"""Protocol smoke fixtures. Never invokes returned URIs or runs the GUI host."""
import argparse,json,pathlib,subprocess,sys
p=argparse.ArgumentParser();p.add_argument('--release',action='store_true');p.add_argument('--github',action='store_true');p.add_argument('--binary-dir',type=pathlib.Path);args=p.parse_args()
root=pathlib.Path(__file__).resolve().parents[1]
binary_dir=args.binary_dir or root/'target'/('release' if args.release else 'debug')
def probe(name,config,actions):
 messages=[dict(type='init',protocol_version=1,config=config)]+[dict(type='invoke',action_id=action,value=value) for action,value in actions]+[dict(type='shutdown')]
 result=subprocess.run([str(binary_dir/(name+'.exe'))],input=''.join(json.dumps(m)+'\n' for m in messages),capture_output=True,text=True,timeout=30)
 assert result.returncode==0,result.stderr
 outputs=[json.loads(line) for line in result.stdout.splitlines()]
 assert outputs[0]==dict(type='ready',protocol_version=1)
 assert any(m['type']=='view' for m in outputs)
 assert not any(m['type']=='error' for m in outputs),outputs
 return outputs
sample=probe('still-island-plugin-developer',{},[('increment',None)]*12+[('notify',None),('toggle',True),('input','Fixture text')])
assert any(m['type']=='activity' for m in sample)
for message in sample:
 if message['type']=='view':
  progress=next(n for n in message['root']['children'] if n['id']=='progress')
  assert 0 <= progress['value'] <= 1
print('PASS developer: handshake, updates, 12 progress actions, activity, toggle, input')
notes=probe('still-island-plugin-obsidian',{'vault':'Fixture Vault'},[('text','Hello & goodbye'),('capture',None),('daily',None)])
uris=[m['open_uri'] for m in notes if m['type']=='action_result' and m.get('open_uri')]
assert any('content=Hello%20%26%20goodbye' in u for u in uris)
assert any(u.startswith('obsidian://daily?') for u in uris)
print('PASS Obsidian: encoded note URI and daily-note URI; neither URI executed')
if args.github:
 github=probe('still-island-plugin-github',{'repository':'rust-lang/rust','monitor':False},[('open',None)])
 view=next(m for m in github if m['type']=='view')
 assert 'GitHub returned' not in json.dumps(view) and 'connection failed' not in json.dumps(view)
 assert any(m.get('open_uri','').startswith('https://github.com/rust-lang/rust/actions/runs/') for m in github if m.get('open_uri'))
 print('PASS GitHub: public live read-only workflow API and run URL; URL not opened')

missing=probe('still-island-plugin-obsidian',{},[('daily',None),('text','Still responsive')])
failed=[m for m in missing if m['type']=='action_result' and not m['success']]
assert len(failed)==1 and failed[0]['action_id']=='daily' and failed[0]['open_uri'] is None
assert any(m.get('action_id')=='text' and m.get('success') for m in missing)
invalid=probe('still-island-plugin-github',{'repository':'invalid','monitor':False},[('refresh',None),('open',None)])
assert {m['action_id'] for m in invalid if m['type']=='action_result' and not m['success']}=={'refresh','open'}
print('PASS recoverable errors: missing vault and invalid repository acknowledge actions; process remains responsive')

# The account-limit integration uses a disposable offline stdio fixture only.
subprocess.run([sys.executable,str(root/'scripts/test-codex-usage.py'),'--binary-dir',str(binary_dir)],check=True,timeout=60)
