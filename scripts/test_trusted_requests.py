# Copyright (C) 2026 The Suhina contributors
# SPDX-License-Identifier: GPL-3.0-or-later

"""Run the shipped privileged workflow scripts against fake GitHub APIs, never the network."""

import json
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parent.parent


def script(name, step):
    text = (ROOT / '.github/workflows' / name).read_text(encoding='utf-8')
    text = text.split(step, 1)[1].split('          script: |\n', 1)[1]
    lines = []
    for line in text.splitlines():
        if line and not line.startswith('            '):
            break
        lines.append(line[12:] if line else '')
    return '\n'.join(lines)


HARNESS = r"""
const fs = require('fs');
const input = JSON.parse(fs.readFileSync(0, 'utf8'));
const sha = 'a'.repeat(40);
const user = {login: 'Trusted', id: 7, type: 'User'};
const bot = {login: 'suhina-bot[bot]', type: 'Bot'};
const comment = {id: 12, body: '/close', user, updated_at: '2026-10-22T10:00:00Z',
  issue_url: 'https://api.github.com/repos/owner/repo/issues/4', ...input.comment};
const context = {repo: {owner: 'owner', repo: 'repo'}, actor: 'suhina-bot[bot]',
  eventName: 'pull_request', payload: {action: 'labeled', comment,
    issue: {number: 4, ...input.issue}, pull_request: {number: 4, head: {sha}}}};
const outputs = {}, calls = [];
const core = {setOutput: (k,v) => outputs[k]=v, info: () => {}};
process.env.TRUSTED_CONTRIBUTORS = input.trusted ?? 'other, @TRUSTED';
process.env.REQUEST = JSON.stringify(input.request || {command:'close',actor:'Trusted',comment:12,number:4});
const comments = (input.comments || []).map(c => ({user:bot, ...c}));
const record = name => async args => { calls.push({name,args}); return {data:{number:99}}; };
const github = {rest:{issues:{
  getComment: async () => {if(input.deleted) throw {status:404}; return {data: input.current || comment};},
  getLabel: async () => ({data:{}}),
  listComments: 'comments', listForRepo:'issues',
  createComment: record('comment'), create: record('create'), update: record('update'),
  removeLabel: record('removeLabel'), addLabels: record('addLabels'),
}, pulls:{get:async () => ({data:{state:'open',head:{sha:input.head || sha}, ...input.pr}})}},
  paginate: async name => name === 'comments' ? comments : (input.issues || []).map(i=>({user:bot,...i}))};
(async () => {
  let error = null, result = null;
  try {result = await new (Object.getPrototypeOf(async()=>{}).constructor)('github','context','core',input.script)(github,context,core);}
  catch(e) {error = e.message || String(e);}
  process.stdout.write(JSON.stringify({outputs,calls,error,result}));
})();
"""


class TrustedRequests(unittest.TestCase):
    def run_script(self, source, **kwargs):
        proc = subprocess.run(['node', '-e', HARNESS], input=json.dumps({'script': source, **kwargs}),
                              text=True, capture_output=True, check=True)
        return json.loads(proc.stdout)

    def check(self, **kwargs):
        return self.run_script(script('trusted-requests.yml', '- name: Check request'), **kwargs)

    def act(self, **kwargs):
        return self.run_script(script('trusted-requests.yml', '- name: Carry out request'), **kwargs)

    def test_roles(self):
        self.assertIn('request', self.check()['outputs'])
        self.assertIn('request', self.check(trusted='', comment={'user': {'login':'juhamikael','id':7,'type':'User'}})['outputs'])
        self.assertIsNotNone(self.check(trusted='')['error'])
        self.assertIsNotNone(self.check(comment={'user': {'login':'Trusted','id':7,'type':'Bot'}})['error'])
        self.assertEqual(self.check(comment={'body':'/ready'})['outputs'], {})

    def test_mutated_request(self):
        for current in [{'body':'/reopen','user':{'id':7}}, {'body':'/close','user':{'id':8}}]:
            self.assertIsNotNone(self.check(current=current)['error'])
        self.assertIsNotNone(self.check(deleted=True)['error'])

    def test_commands_and_targets(self):
        for cmd in ['/close', '/reopen']:
            self.assertIn('request', self.check(comment={'body':cmd})['outputs'])
            self.assertIsNotNone(self.check(comment={'body':cmd}, issue={'pull_request':{}})['error'])
        self.assertIsNotNone(self.check(comment={'body':'/close\n@someone'})['error'])
        self.assertIsNotNone(self.check(comment={'body':'/test-build'})['error'])
        result = self.check(comment={'body':'/test-build'}, issue={'pull_request':{}})
        self.assertEqual(json.loads(result['outputs']['request'])['sha'], 'a'*40)
        self.assertIsNotNone(self.check(comment={'body':'/test-build'}, issue={'pull_request':{}}, pr={'state':'closed'})['error'])

    def test_issue_input_and_spoofed_identity(self):
        valid = {'title':'Title','body':'Body','type':'bug','labels':['ui']}
        result = self.check(comment={'body':'/open-issue\n'+json.dumps(valid)})
        self.assertEqual(json.loads(result['outputs']['request'])['actor'], 'Trusted')
        for patch in [{'author':'juhamikael'}, {'labels':['ready to merge']}, {'title':''},
                      {'body':'x'*60001}, {'type':'__proto__'}, {'labels':'ui'}]:
            self.assertIsNotNone(self.check(comment={'body':'/open-issue\n'+json.dumps(valid|patch)})['error'])
        self.assertIsNotNone(self.check(comment={'body':'/open-issue\nnot JSON'})['error'])

    def test_action_attribution_and_replay(self):
        result = self.act()
        self.assertIsNone(result['error'])
        self.assertEqual(result['calls'][0]['args']['state'], 'closed')
        self.assertIn('On behalf of @Trusted',result['calls'][1]['args']['body'])
        completed = {'body':'Done\n<!-- trusted-request 12 -->'}
        self.assertEqual(self.act(comments=[completed])['calls'], [])
        forged = completed | {'user':{'login':'attacker','type':'User'}}
        self.assertEqual(len(self.act(comments=[forged])['calls']), 2)
        reopened = self.act(request={'command':'reopen','actor':'Trusted','comment':12,'number':4})
        self.assertEqual(reopened['calls'][0]['args']['state'],'open')

    def test_creation_recovers_after_partial_failure(self):
        request = {'command':'open-issue','actor':'Trusted','comment':12,'number':4,
                   'issue':{'title':'Title','body':'Body','type':'Bug','labels':['bug']}}
        result = self.act(request=request)
        self.assertIn('On behalf of @Trusted',result['calls'][0]['args']['body'])
        recovered = self.act(request=request, issues=[{'number':99,'body':'Body\n<!-- trusted-request 12 -->'}])
        self.assertEqual([c['name'] for c in recovered['calls']], ['comment'])
        self.assertIn('#99',recovered['calls'][0]['args']['body'])

    def test_build_binds_head(self):
        request = {'command':'test-build','actor':'Trusted','comment':12,'number':4,'sha':'a'*40}
        result = self.act(request=request)
        self.assertEqual([c['name'] for c in result['calls']], ['comment','removeLabel','addLabels','comment'])
        self.assertIn('<!-- trusted-build '+ 'a'*40, result['calls'][0]['args']['body'])
        self.assertEqual(self.act(request=request,head='b'*40)['calls'], [])
        self.assertEqual(self.act(request=request,pr={'state':'closed'})['calls'], [])
        old = {'body':'<!-- trusted-build '+ 'b'*40 +' Trusted 12 -->'}
        self.assertEqual(self.act(request=request,comments=[old])['calls'], [])

    def test_build_proof_in_both_workflows(self):
        sources = [script('test-build.yml','- name: Authorize the labelled commit'),
                   script('test-build-links.yml','id: link')]
        helpers = []
        for source in sources:
            helper = 'const trusted =' + source.split('const trusted =',1)[1].split('\n};',1)[0] + '\n};'
            helpers.append(helper)
        self.assertEqual(helpers[0],helpers[1])
        proof = {'body':'<!-- trusted-build '+'a'*40+' Trusted 12 -->','created_at':'2026-10-22T10:01:00Z'}
        code = helpers[0]+"\nreturn authorizedBuild(context.actor,4,'a'.repeat(40),'2026-10-22T10:02:00Z');"
        valid = {'comment':{'body':'/test-build'},'comments':[proof]}
        self.assertTrue(self.run_script(code,**valid)['result'])
        for changes in [ {'trusted':''}, {'deleted':True}, {'comments':[]},
                         {'comments':[proof|{'user':{'login':'attacker','type':'User'}}]},
                         {'comments':[proof|{'created_at':'2026-10-22T10:03:00Z'}]},
                         {'comments':[proof|{'body':proof['body'].replace('a'*40,'b'*40)}]},
                         {'comment':{'body':'/test-build','updated_at':'2026-10-22T10:03:00Z'}},
                         {'comment':{'body':'/test-build','issue_url':'https://api.github.com/repos/owner/repo/issues/5'}},
                         {'comment':{'body':'/test-build','user':{'login':'attacker','type':'User'}}},
                         {'comment':{'body':'/close'}} ]:
            self.assertFalse(self.run_script(code,**(valid|changes))['result'])

    def test_build_gate_rejects_missing_authorization(self):
        source = script('test-build.yml', '- name: Authorize the labelled commit')
        self.assertIsNotNone(self.run_script(source)['error'])
        proof = {'body':'<!-- trusted-build '+'a'*40+' Trusted 12 -->','created_at':'2026-10-22T10:01:00Z'}
        self.assertIsNone(self.run_script(source, comment={'body':'/test-build'}, comments=[proof])['error'])


if __name__ == '__main__':
    unittest.main()
