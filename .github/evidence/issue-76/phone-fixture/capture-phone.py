#!/usr/bin/env python3
"""Render the production React Native screen in a fixture-only native dev runtime.
Requires an already-built SDK-57 Lorca development .app for arm64 iOS Simulator.
Only the supplied runtime copy is changed; the caller supplies a dedicated test device.
"""
import argparse, os, pathlib, plistlib, shutil, subprocess, time
HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[3]
parser = argparse.ArgumentParser()
parser.add_argument('--runtime-app', type=pathlib.Path, required=True)
parser.add_argument('--device', required=True)
parser.add_argument('--scenario', choices=['active','followups','empty','before'], default='active')
args = parser.parse_args()
NATIVE = HERE/'.native'; NATIVE.mkdir(exist_ok=True)
def run(command, **kwargs):
    return subprocess.run(command,check=True,**kwargs)
env = os.environ.copy(); env.update(CI='1',EXPO_OFFLINE='1')
(HERE/'scenario.ts').write_text("export const initialScenario = '"+args.scenario+"';\n")
if args.scenario == 'before':
    env['LORCA_CAPTURE_BEFORE']='1'
    old = subprocess.check_output(['git','show','ac1d9b7:mobile/app/attention.tsx'],cwd=ROOT,text=True)
    old = old.replace('\"../', '\"'+str(ROOT/'mobile')+'/')
    (NATIVE/'attention-before.tsx').write_text(old)
run(['node_modules/.bin/expo','export:embed','--entry-file','../../../../mobile/node_modules/expo-router/entry.js',
     '--platform','ios','--dev','false','--minify','false','--max-workers','2','--bundle-output','.native/main.jsbundle','--assets-dest','.native/assets'],cwd=HERE,env=env)
app = NATIVE/'LorcaAttentionFixture.app'
if not app.exists(): run(['ditto',str(args.runtime_app),str(app)])
shutil.copy2(NATIVE/'main.jsbundle',app/'main.jsbundle')
run(['ditto',str(NATIVE/'assets/assets'),str(app/'assets')])
info = plistlib.loads((app/'Info.plist').read_bytes())
info['CFBundleDisplayName']='Lorca UI Fixture'; info['EXDevClientEmbeddedBundle']=True
(app/'Info.plist').write_bytes(plistlib.dumps(info))
run(['xcrun','--sdk','iphonesimulator','clang','-dynamiclib','-fobjc-arc','-target','arm64-apple-ios27.0-simulator',
     '-framework','Foundation',str(HERE/'load-embedded.m'),'-o',str(app/'AutoFixtureLoader.dylib')])
run(['codesign','--force','--deep','--sign','-',str(app)])
run(['xcrun','simctl','install',args.device,str(app)])
installed = subprocess.check_output(['xcrun','simctl','get_app_container',args.device,info['CFBundleIdentifier'],'app'],text=True).strip()
launch_env=os.environ.copy();launch_env['SIMCTL_CHILD_DYLD_INSERT_LIBRARIES']=installed+'/AutoFixtureLoader.dylib'
run(['xcrun','simctl','launch','--terminate-running-process',args.device,info['CFBundleIdentifier']],env=launch_env)
# Bounded wait for native SDK startup, fixture routing and menu dismissal.
time.sleep(15)
run(['xcrun','simctl','io',args.device,'screenshot',str(HERE.parent/f'phone-attention-{args.scenario}.png')])
(HERE/'scenario.ts').write_text("export const initialScenario = 'active';\n")
print('Fixture-only native capture:',args.scenario)
