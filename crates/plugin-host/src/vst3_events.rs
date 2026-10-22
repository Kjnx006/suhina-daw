// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Notes and controllers for a VST3 instrument. Notes go to `process` as an `IEventList`;
//! controllers, pitch bend and channel pressure (aftertouch, #812) are not events in VST3 but
//! parameters, found through the controller's `IMidiMapping`, so they go as parameter changes
//! ([`crate::vst3_params`]). Both lists are made before processing starts, so the audio thread
//! does not allocate.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use suhina_ipc::{Event as LinkEvent, EventKind, MAX_EVENTS};
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{Class, ComPtr, ComWrapper};

/// The controller numbers a mapping covers: MIDI controllers 0–127, then channel aftertouch
/// (128) and pitch bend (129), as VST3 numbers them.
const CONTROLLERS: usize = 130;

/// Which parameter MIDI controllers and pitch bend move, from the plug-in's `IMidiMapping`
/// (event bus 0). Only the first MIDI channel is asked, and its mapping serves every channel:
/// some plug-ins fail when asked about another channel, and the
/// notes a project plays come on the first.
pub(crate) struct MidiMapping {
    table: Arc<[Option<ParamID>; CONTROLLERS]>,
    shown: Shown,
}

/// No value sent since the main thread last looked: not the bits of any value in 0 to 1.
const UNCHANGED: u64 = u64::MAX;

/// The latest value the audio thread gave each mapped controller, for the main thread to give
/// the edit controller too ([`Shown::take`]): VST3 leaves keeping the controller in step with
/// the processor to the host, and the editor draws what the controller holds, so without it a
/// plug-in's on-screen wheels stay put while the sound follows a MIDI controller's. Set on the
/// audio thread and taken on the main thread without a lock.
#[derive(Clone)]
pub(crate) struct Shown {
    table: Arc<[Option<ParamID>; CONTROLLERS]>,
    values: Arc<[AtomicU64; CONTROLLERS]>,
}

impl Shown {
    /// Notes `value` as the latest of controller `number`. Real-time safe.
    fn set(&self, number: usize, value: ParamValue) {
        if let Some(slot) = self.values.get(number) {
            slot.store(value.to_bits(), Ordering::Release);
        }
    }

    /// The parameters whose controller moved since the last call, each with its latest value.
    pub(crate) fn take(&self) -> impl Iterator<Item = (ParamID, ParamValue)> + '_ {
        self.values.iter().zip(self.table.iter()).filter_map(|(slot, id)| {
            let bits = slot.swap(UNCHANGED, Ordering::AcqRel);
            if bits == UNCHANGED {
                return None;
            }
            (*id).map(|id| (id, f64::from_bits(bits)))
        })
    }
}

impl MidiMapping {
    /// Asks `controller` for its assignments; a plug-in without a mapping maps nothing.
    pub(crate) fn of(controller: Option<&ComPtr<IEditController>>) -> Self {
        let mut table = Box::new([None; CONTROLLERS]);
        let Some(mapping) = controller.and_then(|c| c.cast::<IMidiMapping>()) else {
            // Without it a VST3 plug-in cannot be sent controllers or pitch bend at all.
            suhina_plugins::trace!(
                "vst3 midi mapping: {}",
                if controller.is_some() { "the controller has no IMidiMapping" } else { "no controller" }
            );
            return Self::with(table);
        };
        for (number, slot) in table.iter_mut().enumerate() {
            let mut id: ParamID = 0;
            // SAFETY: a live controller writing one id.
            let found = unsafe { mapping.getMidiControllerAssignment(0, 0, number as CtrlNumber, &mut id) };
            if found == kResultOk {
                *slot = Some(id);
            }
        }
        let mapped = table.iter().filter(|s| s.is_some()).count();
        let at = |number: usize| table.get(number).copied().flatten();
        suhina_plugins::trace!(
            "vst3 midi mapping: {mapped} of {CONTROLLERS} mapped; mod wheel {:?}, aftertouch {:?}, pitch bend {:?}",
            at(1),
            at(128),
            at(129)
        );
        Self::with(table)
    }

    fn with(table: Box<[Option<ParamID>; CONTROLLERS]>) -> Self {
        let table: Arc<[Option<ParamID>; CONTROLLERS]> = Arc::from(table);
        let values = Arc::new(std::array::from_fn(|_| AtomicU64::new(UNCHANGED)));
        Self { shown: Shown { table: Arc::clone(&table), values }, table }
    }

    /// Where the main thread finds the values it sent, to show them in the editor.
    pub(crate) fn shown(&self) -> Shown {
        self.shown.clone()
    }

    /// The parameter change a controller, pitch bend, channel pressure or parameter event
    /// makes: id, frame, normalized value. A controller's is noted for the editor too
    /// ([`Shown`]). Real-time safe.
    pub(crate) fn change(&self, event: &LinkEvent) -> Option<(ParamID, i32, ParamValue)> {
        if let EventKind::Param { id, value } = event.kind {
            // A parameter's own value, kept within VST3's normalized range.
            return Some((id, event.frame as i32, f64::from(value).clamp(0.0, 1.0)));
        }
        let (number, id, value) = self.controller(event)?;
        self.shown.set(number, value);
        Some((id, event.frame as i32, value))
    }

    /// The controller number a controller, pitch bend or channel pressure event moves, the
    /// parameter mapped to it and its normalized value; `None` for anything else or a
    /// controller nothing is mapped to.
    pub(crate) fn controller(&self, event: &LinkEvent) -> Option<(usize, ParamID, ParamValue)> {
        let (number, value) = match event.kind {
            EventKind::Controller { number, value, .. } => (usize::from(number), f64::from(value)),
            // -1 to 1 becomes 0 to 1, the centre at one half.
            EventKind::PitchBend { value, .. } => (129, (f64::from(value) + 1.0) / 2.0),
            // Channel aftertouch, as VST3 numbers it (#812).
            EventKind::Pressure { value, .. } => (128, f64::from(value)),
            EventKind::Param { .. }
            | EventKind::NoteOn { .. }
            | EventKind::NoteOff { .. }
            | EventKind::Transport { .. }
            | EventKind::Tempo { .. }
            | EventKind::Beat { .. }
            | EventKind::Bar { .. }
            | EventKind::Cycle { .. }
            | EventKind::CycleSeconds { .. } => return None,
        };
        let id = (*self.table.get(number)?)?;
        Some((number, id, value.clamp(0.0, 1.0)))
    }
}

/// One block's notes.
struct EventList {
    events: RefCell<Vec<Event>>,
}

impl Class for EventList {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for EventList {
    unsafe fn getEventCount(&self) -> int32 {
        self.events.try_borrow().map_or(0, |e| e.len() as int32)
    }

    unsafe fn getEvent(&self, index: int32, event: *mut Event) -> tresult {
        let Ok(events) = self.events.try_borrow() else { return kResultFalse };
        let Some(found) = usize::try_from(index).ok().and_then(|i| events.get(i)) else { return kInvalidArgument };
        if event.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the plug-in passes an event to fill; `found` is a plain C struct.
        unsafe { *event = *found };
        kResultOk
    }

    unsafe fn addEvent(&self, _event: *mut Event) -> tresult {
        kResultFalse
    }
}

/// The audio thread's side: the list handed to `process` as `inputEvents`.
pub(crate) struct BlockEvents {
    list: ComWrapper<EventList>,
    pointer: Option<ComPtr<IEventList>>,
}

impl BlockEvents {
    /// Made before processing starts, with room for [`MAX_EVENTS`] notes.
    pub(crate) fn new() -> Self {
        let list = ComWrapper::new(EventList { events: RefCell::new(Vec::with_capacity(MAX_EVENTS)) });
        let pointer = list.to_com_ptr::<IEventList>();
        Self { list, pointer }
    }

    /// The block's notes as VST3 events; null when there are none. Real-time safe.
    pub(crate) fn fill(&self, events: &[LinkEvent]) -> *mut IEventList {
        let Ok(mut list) = self.list.events.try_borrow_mut() else { return std::ptr::null_mut() };
        list.clear();
        for event in events.iter().take(MAX_EVENTS) {
            if let Some(converted) = note(event) {
                list.push(converted);
            }
        }
        if list.is_empty() {
            std::ptr::null_mut()
        } else {
            self.pointer.as_ref().map_or(std::ptr::null_mut(), ComPtr::as_ptr)
        }
    }
}

/// A note-on or note-off as a VST3 event on event bus 0.
fn note(event: &LinkEvent) -> Option<Event> {
    let mut out = Event {
        busIndex: 0,
        sampleOffset: event.frame as i32,
        ppqPosition: 0.0,
        flags: Event_::EventFlags_::kIsLive as uint16,
        r#type: 0,
        __field0: Event__type0 {
            noteOn: NoteOnEvent { channel: 0, pitch: 0, tuning: 0.0, velocity: 0.0, length: 0, noteId: -1 },
        },
    };
    match event.kind {
        EventKind::NoteOn { channel, key, velocity } => {
            out.r#type = Event_::EventTypes_::kNoteOnEvent as uint16;
            out.__field0 = Event__type0 {
                noteOn: NoteOnEvent {
                    channel: int16::from(channel),
                    pitch: int16::from(key),
                    tuning: 0.0,
                    velocity,
                    length: 0,
                    noteId: event.note,
                },
            };
        }
        EventKind::NoteOff { channel, key, velocity } => {
            out.r#type = Event_::EventTypes_::kNoteOffEvent as uint16;
            out.__field0 = Event__type0 {
                noteOff: NoteOffEvent {
                    channel: int16::from(channel),
                    pitch: int16::from(key),
                    velocity,
                    noteId: event.note,
                    tuning: 0.0,
                },
            };
        }
        EventKind::Controller { .. }
        | EventKind::PitchBend { .. }
        | EventKind::Pressure { .. }
        | EventKind::Param { .. }
        | EventKind::Transport { .. }
        | EventKind::Tempo { .. }
        | EventKind::Beat { .. }
        | EventKind::Bar { .. }
        | EventKind::Cycle { .. }
        | EventKind::CycleSeconds { .. } => return None,
    }
    Some(out)
}
