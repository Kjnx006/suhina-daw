// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The grid of a timeline at one zoom: the lines drawn and the beats things snap to. The
//! arrangement, the piano roll and the automation lanes all use it, so snapping is the same
//! wherever a user edits on a timeline (`docs/spec/arrangement-editing.md` › The grid and
//! `docs/spec/ui-clip-editor.md` › Grid and snapping). Each view keeps its own grid setting and
//! turns it into a step; only the arithmetic is here.

/// Beats closer than this are the same grid line.
const EPSILON: f64 = 1e-9;

/// The finest note value an adaptive grid reaches, in beats (1/32).
const FINEST: f64 = 0.125;

/// How many times an adaptive grid doubles a bar when zoomed far out, at most.
const MAX_BAR_DOUBLINGS: i32 = 16;

/// How many bars a grid coarsens to at most when zoomed far out.
const MAX_BARS: f64 = 1024.0;

/// A timeline's grid at one zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineGrid {
    /// Beats between lines inside a bar; a whole number of bars when it is a bar or more.
    pub step: f64,
    /// Beats in a bar: lines start again at every bar line.
    pub bar_beats: f64,
    /// Whether things snap to the lines; without, the lines are still drawn.
    pub snaps: bool,
}

impl TimelineGrid {
    /// The grid for a chosen step of `chosen` beats (0 or less: no snapping, lines by the zoom
    /// alone), at `beat_width` points a beat with bars of `bar_beats` beats. Lines are the chosen
    /// step apart, or coarser where they would be closer than `min_spacing` points: a straight
    /// step doubles up to the beat, a triplet goes to the beat, then the beat goes to the bar and
    /// bars double. So the lines always land on the bar lines.
    pub fn new(chosen: f64, beat_width: f32, bar_beats: f64, min_spacing: f32) -> Self {
        let bar_beats = bar_length(bar_beats);
        let snaps = chosen.is_finite() && chosen > 0.0;
        let mut step = if snaps { chosen } else { 1.0 };
        // A note value longer than a bar that is not a whole number of bars (a half note in
        // 3/8) is drawn on the bar lines instead, so lines stay on the bar.
        if step > bar_beats + EPSILON && !is_whole(step / bar_beats) {
            step = bar_beats;
        }
        let points = f64::from(beat_width.max(f32::MIN_POSITIVE));
        let min = f64::from(min_spacing);
        while step * points < min && step < MAX_BARS * bar_beats {
            step = if step >= bar_beats - EPSILON {
                step * 2.0
            } else if step < 1.0 - EPSILON && is_whole((1.0 / step).log2()) {
                // A straight note value finer than a beat: 1/32, 1/16, 1/8, 1/4.
                (step * 2.0).min(1.0)
            } else if step < 1.0 - EPSILON {
                // A triplet finer than a beat goes to the beat, whose lines are on the bar.
                1.0
            } else {
                bar_beats
            };
            // A bar shorter than a beat (2/8): the beat is coarser than the bar, so go by bars.
            if step > bar_beats + EPSILON && !is_whole(step / bar_beats) {
                step = bar_beats;
            }
        }
        Self { step, bar_beats, snaps }
    }

    /// Whether the step is one bar or more, so lines are only on bar lines.
    fn in_bars(&self) -> bool {
        self.step >= self.bar_beats - EPSILON
    }

    /// The grid line nearest to `beat`, or with `down` the line at or before it (the start of
    /// the grid cell it is in); `beat` itself with `free` (Alt held) or a grid that does not
    /// snap. Lines inside a bar are counted from its bar line, so a step that does not fill the
    /// bar (two beats in 3/4) starts again at each bar. Never before the start.
    pub fn place(&self, beat: f64, free: bool, down: bool) -> f64 {
        if !beat.is_finite() {
            return 0.0;
        }
        let beat = beat.max(0.0);
        if free || !self.snaps || self.step <= 0.0 {
            return beat;
        }
        if self.in_bars() {
            let lines = beat / self.step;
            return (if down { (lines + EPSILON).floor() } else { lines.round() }) * self.step;
        }
        let bar_start = (beat / self.bar_beats + EPSILON).floor() * self.bar_beats;
        if down {
            return bar_start + ((beat - bar_start) / self.step + EPSILON).floor() * self.step;
        }
        // Inside a bar: lines from the bar line on, and the next bar line.
        let next_bar = bar_start + self.bar_beats;
        let on_step = bar_start + ((beat - bar_start) / self.step).round() * self.step;
        if on_step >= next_bar - EPSILON || (next_bar - beat) < (beat - on_step).abs() { next_bar } else { on_step }
    }

    /// The first line after `beat`, as [`TimelineGrid::lines`] draws them.
    pub fn next_line(&self, beat: f64) -> f64 {
        let cell = self.place(beat, false, true);
        let step = if self.step > 0.0 { self.step } else { 1.0 };
        if self.in_bars() {
            return cell + step;
        }
        let bar_start = (cell / self.bar_beats + EPSILON).floor() * self.bar_beats;
        (cell + step).min(bar_start + self.bar_beats)
    }

    /// The last line before `beat`, never before the start.
    pub fn previous_line(&self, beat: f64) -> f64 {
        if !beat.is_finite() || beat <= EPSILON {
            return 0.0;
        }
        // Just before `beat`, so a line on it is not the one before it.
        self.place(beat - 1e-6, false, true)
    }

    /// The beats of the lines from `first` to `last` (both included), each with whether it is a
    /// bar line. At most `limit` lines, so a bad zoom can never draw without end.
    pub fn lines(&self, first: f64, last: f64, limit: usize) -> Vec<(f64, bool)> {
        let mut lines = Vec::new();
        if !(first.is_finite() && last.is_finite()) || self.step <= 0.0 {
            return lines;
        }
        let first = first.max(0.0);
        if self.in_bars() {
            let mut n = (first / self.step).floor();
            while lines.len() < limit {
                let beat = n * self.step;
                if beat > last {
                    break;
                }
                lines.push((beat, true));
                n += 1.0;
            }
            return lines;
        }
        let mut bar = (first / self.bar_beats).floor();
        'bars: while lines.len() < limit {
            let bar_start = bar * self.bar_beats;
            if bar_start > last {
                break;
            }
            if bar_start >= first - EPSILON {
                lines.push((bar_start, true));
            }
            let mut k = ((first - bar_start) / self.step).ceil().max(1.0);
            loop {
                let beat = bar_start + k * self.step;
                if beat >= bar_start + self.bar_beats - EPSILON {
                    break;
                }
                if lines.len() >= limit || beat > last {
                    break 'bars;
                }
                lines.push((beat, false));
                k += 1.0;
            }
            bar += 1.0;
        }
        lines
    }
}

/// An adaptive grid's step at `beat_width` points a beat: the finest power of two note value
/// (1/32 up to the last one shorter than a bar; two thirds as long with `triplet`), then 1 bar,
/// 2 bars, 4 bars and on, whose lines are at least `spacing` points apart. An unusable zoom (not
/// a positive number) gives one bar.
pub fn adaptive_step(spacing: f64, triplet: bool, beat_width: f32, bar_beats: f64) -> f64 {
    let bar = bar_length(bar_beats);
    if !(beat_width.is_finite() && beat_width > 0.0) {
        return bar;
    }
    let points = f64::from(beat_width);
    let third = if triplet { 2.0 / 3.0 } else { 1.0 };
    // The note values shorter than a bar: 1/32, 1/16, ... at most five of them (to 1/2).
    let mut value = FINEST;
    for _ in 0..5 {
        if value >= bar {
            break;
        }
        if value * third * points >= spacing {
            return value * third;
        }
        value *= 2.0;
    }
    let mut step = bar;
    for _ in 0..MAX_BAR_DOUBLINGS {
        if step * points >= spacing {
            return step;
        }
        step *= 2.0;
    }
    step
}

/// Beats in a bar, or 4 when `bar_beats` is not a positive number.
pub fn bar_length(bar_beats: f64) -> f64 {
    if bar_beats.is_finite() && bar_beats > 0.0 { bar_beats } else { 4.0 }
}

fn is_whole(value: f64) -> bool {
    value.is_finite() && (value - value.round()).abs() < 1e-6
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid of `step` beats in bars of `bar_beats`, at `beat_width` points a beat, with lines
    /// at least 8 points apart.
    fn grid(step: f64, beat_width: f32, bar_beats: f64) -> TimelineGrid {
        TimelineGrid::new(step, beat_width, bar_beats, 8.0)
    }

    /// The lines from `first` to `last`, in thousandths of a beat.
    fn drawn(grid: TimelineGrid, first: f64, last: f64) -> Vec<f64> {
        grid.lines(first, last, 1000).into_iter().map(|(b, _)| (b * 1000.0).round()).collect()
    }

    /// Whether every place `grid` snaps to between beats 0 and `last` is a drawn line.
    fn snaps_to_drawn_lines(grid: TimelineGrid, last: f64) -> bool {
        let lines = drawn(grid, 0.0, last + grid.step.max(grid.bar_beats));
        (0..(last * 100.0) as usize).map(|n| n as f64 / 100.0).all(|beat| {
            [grid.place(beat, false, false), grid.place(beat, false, true)]
                .iter()
                .all(|placed| lines.contains(&(placed * 1000.0).round()))
        })
    }

    #[test]
    fn a_straight_step_doubles_to_the_beat_then_goes_to_bars() {
        assert_eq!(grid(0.25, 32.0, 4.0).step, 0.25);
        assert_eq!(grid(0.25, 31.0, 4.0).step, 0.5);
        assert_eq!(grid(0.25, 15.0, 4.0).step, 1.0);
        assert_eq!(grid(0.25, 7.0, 4.0).step, 4.0);
        assert_eq!(grid(0.25, 1.9, 4.0).step, 8.0);
        assert_eq!(grid(0.0, 400.0, 4.0), TimelineGrid { step: 1.0, bar_beats: 4.0, snaps: false });
        // A half note in 3/4 is not on the bar: once too narrow it goes to the bar.
        assert_eq!(grid(2.0, 5.0, 3.0).step, 2.0);
        assert_eq!(grid(2.0, 3.0, 3.0).step, 3.0);
        // A broken bar length is four beats; a broken zoom coarsens to the most bars and stops.
        assert_eq!(grid(1.0, 400.0, f64::NAN).bar_beats, 4.0);
        assert_eq!(grid(1.0, f32::NAN, 4.0).step, 4096.0);
    }

    #[test]
    fn places_snap_to_the_lines_inside_each_bar_unless_free() {
        let sixteenths = grid(0.25, 1000.0, 4.0);
        assert_eq!(sixteenths.place(2.13, false, false), 2.25);
        assert_eq!(sixteenths.place(2.13, true, false), 2.13);
        assert_eq!(sixteenths.place(-1.0, false, false), 0.0);
        assert_eq!(sixteenths.place(f64::NAN, false, false), 0.0);
        assert_eq!(sixteenths.place(1.45, false, true), 1.25);
        // Triplet lines start again at each bar line: in 3/4 a half-note triplet (4/3 beats)
        // has lines at 0, 4/3 and 8/3, then the bar line at 3.
        let triplets = grid(4.0 / 3.0, 400.0, 3.0);
        assert!((triplets.place(1.5, false, false) - 4.0 / 3.0).abs() < 1e-12);
        assert_eq!(triplets.place(2.9, false, false), 3.0);
        assert!((triplets.place(4.4, false, false) - (3.0 + 4.0 / 3.0)).abs() < 1e-12);
        // Bars: on whole steps from the start.
        let two_bars = grid(8.0, 8.0, 4.0);
        assert_eq!(two_bars.place(11.0, false, false), 8.0);
        assert_eq!(two_bars.place(13.0, false, false), 16.0);
    }

    #[test]
    fn down_gives_the_line_before_counted_from_its_bar() {
        // In 3/4 a two-beat step has lines at 0 and 2 in each bar, then the bar line at 3: 3.1
        // is in the cell that starts on the bar line, not at beat 2.
        let halves = grid(2.0, 400.0, 3.0);
        assert_eq!(halves.place(3.1, false, true), 3.0);
        assert_eq!(halves.place(2.9, false, true), 2.0);
        assert_eq!(halves.place(5.5, false, true), 5.0);
        assert_eq!(halves.place(3.1, true, true), 3.1);
        assert_eq!(halves.place(-1.0, false, true), 0.0);
        assert_eq!(grid(8.0, 8.0, 4.0).place(15.9, false, true), 8.0);
    }

    #[test]
    fn the_lines_next_to_a_beat_are_drawn_lines() {
        let halves = grid(2.0, 400.0, 3.0);
        assert_eq!(halves.next_line(0.5), 2.0);
        assert_eq!(halves.next_line(2.0), 3.0, "the bar line, not beat 4");
        assert_eq!(halves.next_line(3.0), 5.0);
        assert_eq!(halves.previous_line(3.0), 2.0);
        assert_eq!(halves.previous_line(5.0), 3.0);
        assert_eq!(halves.previous_line(2.5), 2.0);
        assert_eq!(halves.previous_line(0.0), 0.0);
        let bars = grid(4.0, 8.0, 4.0);
        assert_eq!((bars.next_line(4.0), bars.previous_line(4.0)), (8.0, 0.0));
    }

    #[test]
    fn lines_are_drawn_at_every_step_with_bar_lines_marked() {
        let beats = grid(1.0, 1000.0, 4.0);
        assert_eq!(
            beats.lines(3.5, 6.0, 100),
            vec![(4.0, true), (5.0, false), (6.0, false)],
            "the bar before the view starts the lines"
        );
        assert_eq!(drawn(grid(4.0 / 3.0, 400.0, 3.0), 0.0, 3.0), vec![0.0, 1333.0, 2667.0, 3000.0]);
        assert_eq!(grid(8.0, 8.0, 4.0).lines(1.0, 17.0, 100), vec![(0.0, true), (8.0, true), (16.0, true)]);
        // Never more than asked for.
        assert_eq!(grid(0.125, 1000.0, 4.0).lines(0.0, 1e9, 50).len(), 50);
        assert!(beats.lines(f64::NAN, 4.0, 10).is_empty());
    }

    #[test]
    fn a_triplet_zoomed_out_in_4_4_stays_on_the_bar_lines() {
        // 1/16T at 6 points a line, as the piano roll draws it: a sixth of a beat from 36 points.
        let at = |beat_width: f32| TimelineGrid::new(1.0 / 6.0, beat_width, 4.0, 6.0);
        assert!((at(36.0).step - 1.0 / 6.0).abs() < 1e-12);
        // Narrower: the beat, then the bar, then bars double; never 1/3, 2/3, 4/3 or 8/3.
        assert_eq!(at(35.0).step, 1.0);
        assert_eq!(at(5.0).step, 4.0);
        assert_eq!(at(1.0).step, 8.0);
        for beat_width in [100.0, 35.0, 20.0, 10.0, 5.0, 2.0, 1.0] {
            let grid = at(beat_width);
            assert!(drawn(grid, 0.0, 16.0).contains(&16000.0), "a line on bar 5 at {beat_width}");
            assert!(snaps_to_drawn_lines(grid, 16.0), "{beat_width}");
        }
    }

    #[test]
    fn odd_bars_keep_their_lines_on_the_bar_lines() {
        // 5/4, one beat zoomed out: the bar (5 beats), then 10, not 2, 4 or 8 beats.
        let five = |beat_width: f32| TimelineGrid::new(1.0, beat_width, 5.0, 6.0);
        assert_eq!(five(5.0).step, 5.0);
        assert_eq!(five(1.0).step, 10.0);
        // 5/4 with a two-beat step: lines at 0, 2, 4 and the bar line at 5, then 7, 9, 10.
        let halves = TimelineGrid::new(2.0, 100.0, 5.0, 6.0);
        assert_eq!(drawn(halves, 0.0, 10.0), vec![0.0, 2000.0, 4000.0, 5000.0, 7000.0, 9000.0, 10000.0]);
        // 3/8 (a bar of 1.5 beats), 1/4 triplet: lines at 0, 2/3, 4/3 and on each bar line.
        let in_3_8 = TimelineGrid::new(2.0 / 3.0, 100.0, 1.5, 6.0);
        assert_eq!(drawn(in_3_8, 0.0, 3.0), vec![0.0, 667.0, 1333.0, 1500.0, 2167.0, 2833.0, 3000.0]);
        for grid in [five(5.0), five(1.0), halves, in_3_8, TimelineGrid::new(2.0 / 3.0, 3.0, 1.5, 6.0)] {
            assert!(snaps_to_drawn_lines(grid, 20.0), "{grid:?}");
        }
    }

    #[test]
    fn an_adaptive_step_follows_the_zoom_the_bar_and_triplets() {
        // 24 points apart at least.
        assert_eq!(adaptive_step(24.0, false, 10.0, 4.0), 4.0, "a bar at 10 points per beat");
        assert_eq!(adaptive_step(24.0, false, 40.0, 4.0), 1.0, "a quarter at 40");
        assert_eq!(adaptive_step(24.0, false, 100.0, 4.0), 0.25, "a sixteenth at 100");
        assert_eq!(adaptive_step(24.0, false, 1000.0, 4.0), 0.125, "never finer than 1/32");
        assert_eq!(adaptive_step(24.0, false, 1.0, 4.0), 32.0, "bars double when zoomed far out");
        // In 3/4 the step after the half note is the 3-beat bar.
        assert_eq!(adaptive_step(24.0, false, 10.0, 3.0), 3.0);
        assert_eq!(adaptive_step(24.0, false, 12.0, 3.0), 2.0);
        assert!((adaptive_step(24.0, true, 100.0, 4.0) - 1.0 / 3.0).abs() < 1e-12, "an eighth triplet at 100");
        for beat_width in [f32::NAN, 0.0, -5.0, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(adaptive_step(6.0, false, beat_width, 4.0), 4.0, "{beat_width}");
        }
        assert_eq!(adaptive_step(96.0, false, f32::MIN_POSITIVE, 4.0), 4.0 * 2f64.powi(MAX_BAR_DOUBLINGS));
        assert_eq!(adaptive_step(24.0, false, 40.0, f64::NAN), 1.0, "a broken bar length is 4 beats");
    }
}
