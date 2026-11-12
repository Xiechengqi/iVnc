// Real Electron launch diagnostics, recovery and same-UID sessions. Run as root
// and again as a non-root user with a working Electron sandbox.
// ELECTRON_BIN=/path/to/electron IVNC_BIN=target/debug/ivnc node scripts/launch_smoke.mjs
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, openSync, closeSync, readFileSync, rmSync, writeFileSync, readdirSync, symlinkSync, linkSync, copyFileSync, statSync, chmodSync } from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';
const root=path.resolve(new URL('..',import.meta.url).pathname);
const binary=path.resolve(process.env.IVNC_BIN||path.join(root,'target/debug/ivnc'));
const electron=process.env.ELECTRON_BIN;assert(electron,'Set ELECTRON_BIN');
const work=mkdtempSync(path.join(tmpdir(),'ivnc-launch-smoke-'));
const env={...process.env,HOME:path.join(work,'home'),IVNC_HOME:path.join(work,'ivnc'),XDG_RUNTIME_DIR:path.join(work,'run')};
for(const k of ['HOME','IVNC_HOME','XDG_RUNTIME_DIR'])mkdirSync(env[k],{mode:0o700});
async function port(){const s=net.createServer();await new Promise(r=>s.listen(0,'127.0.0.1',r));const p=s.address().port;await new Promise(r=>s.close(r));return p;}
const httpPort=await port(),controlPort=await port(),base=`http://127.0.0.1:${httpPort}`;
const config=path.join(work,'config.toml');writeFileSync(config,'[audio]\nenabled=false\nsample_rate=48000\nchannels=2\nbitrate=128000\n');
// Package the real Electron fixture so the user's command is just a program
// name, as it is for installed amp-desktop/t3code packages. Isolate resources.
const packaged=path.join(work,'desktop'),binDir=path.join(work,'bin');
mkdirSync(packaged);mkdirSync(binDir);
for(const entry of readdirSync(path.dirname(electron))){
 if(entry==='electron'||entry==='resources')continue;
 symlinkSync(path.join(path.dirname(electron),entry),path.join(packaged,entry));
}
const packagedBinary=path.join(packaged,'fixture-desktop');
try{linkSync(electron,packagedBinary);}catch{copyFileSync(electron,packagedBinary);}
mkdirSync(path.join(packaged,'resources','app'),{recursive:true});
writeFileSync(path.join(packaged,'resources','app','package.json'),JSON.stringify({name:'ivnc-launch-fixture',version:'1.0.0',main:'main.cjs'}));
writeFileSync(path.join(packaged,'resources','app','main.cjs'),`require(${JSON.stringify(path.join(root,'scripts/fixtures/electron-wayland/main.cjs'))});`);
symlinkSync(packagedBinary,path.join(binDir,'fixture-desktop'));
env.PATH=binDir+path.delimiter+process.env.PATH;
if(process.geteuid()!==0){
 // Shell PATH lookup skips files not executable by the current user, even
 // when another permission class has an execute bit. Inspection must too.
 const denied=path.join(work,'not-executable');mkdirSync(denied);
 const shadow=path.join(denied,'fixture-desktop');writeFileSync(shadow,'\x7fELF\0not Electron');chmodSync(shadow,0o601);
 env.PATH=denied+path.delimiter+env.PATH;
}
const fd=openSync(path.join(work,'server.log'),'a',0o600);
const server=spawn(binary,['--config',config,'--http-port',String(httpPort),'--basic-auth-enabled','false','--proxy-enabled','false','--proxy-autostart','false','--width','800','--height','600'],{env,cwd:work,stdio:['ignore',fd,fd]});closeSync(fd);
let appId,passed=0;
const pass=s=>{passed++;console.log(`PASS ${s}`)};
const quote=s=>"'"+s.replaceAll("'","'\\''")+"'";
const command='fixture-desktop';
const rootUser=process.geteuid()===0;
const working=command;
async function api(url,method='GET',body,allowError=false){
 const r=await fetch(base+url,{method,headers:{'Content-Type':'application/json'},body:body?JSON.stringify(body):undefined});
 const d=await r.json();if(!allowError)assert(r.ok,JSON.stringify(d));return d;
}
async function wait(name,fn,ms=25000){const end=Date.now()+ms;let last;while(Date.now()<end){try{const v=await fn();if(v)return v;}catch(e){last=e;}await delay(100);}throw Error(`${name}: ${last||'timed out'}`);}
const details=()=>api(`/api/apps/${appId}`);
const control=()=>fetch(`http://127.0.0.1:${controlPort}/state`).then(r=>r.json());
async function update(exec,desktop_launch){await api(`/api/apps/${appId}`,'PUT',{exec_command:exec,desktop_launch,env_vars:{ELECTRON_CONTROL_PORT:String(controlPort)}});}
async function stop(){await api(`/api/apps/${appId}/stop`,'POST');}
try{
 await wait('server',()=>api('/api/apps/launch-context'));
 const context=await api('/api/apps/launch-context');assert.equal(context.identity.uid,process.geteuid());assert.equal(context.identity.gid,process.getegid());pass('launch context reports actual UID/GID');
 const created=await api('/api/apps','POST',{name:'Launch diagnostic fixture',app_type:'desktop',exec_command:command,desktop_launch:{compatibility:'auto',sandbox:rootUser?'preserve':'auto'},env_vars:{ELECTRON_CONTROL_PORT:String(controlPort)}});appId=created.app.id;
 if(rootUser){
  await api(`/api/apps/${appId}/start`,'POST');
  await wait('root sandbox diagnostic',async()=>(await details()).launch_error?.code==='desktop_root_sandbox');
  await delay(8000);
  const log=readFileSync(path.join(env.IVNC_HOME,'apps',appId,'app.log'),'utf8');
  assert.equal(log.split('Running as root without --no-sandbox is not supported').length-1,1);pass('real Electron root sandbox failure is explained and not automatically retried');
  await update(working,{compatibility:'auto',sandbox:'auto'});
 }
 await api(`/api/apps/${appId}/start`,'POST');
 const state=await wait('Electron window',control);assert.equal(state.session,'wayland');
 await wait('window status',async()=>(await details()).window_ready===true);
 assert.equal((await details()).launch_error,null);
 const proc=await fetch(`http://127.0.0.1:${controlPort}/identity`).then(r=>r.json());
 assert.equal(proc.uid,process.geteuid());assert.equal(proc.cwd,proc.home);
 assert.equal(proc.sandboxDisabled,rootUser);
 assert.equal(proc.ozonePlatform,'wayland');assert.equal(proc.waylandIme,true);assert.equal(proc.textInputVersion,'3');
 const info=(await api(`/api/apps/${appId}/launch-info`)).launch_info;
 assert.equal(info.original_command,command);assert.equal(info.reason,'electron');assert.equal(info.uid,process.geteuid());
 assert.equal(info.executable,packagedBinary);
 assert.equal(info.window_seen,true);assert.equal(statSync(path.join(env.IVNC_HOME,'apps',appId,'launch-info.json')).mode&0o777,0o600);
 assert.equal(info.diagnostic_log,undefined);assert.equal(info.report_path,undefined);
 assert.equal((await details()).exec_command,command);assert(info.added_args.includes('--ozone-platform=wayland'));
 pass('automatic Electron launch adds Wayland and IME parameters without rewriting the saved command');
 if(!rootUser){
  assert.equal(proc.rendererSandboxPreferred,true);
  const status=readFileSync(`/proc/${proc.rendererPid}/status`,'utf8');
  assert.match(status,/^NoNewPrivs:\s+1$/m);assert.match(status,/^Seccomp:\s+2$/m);
 }
 pass(rootUser?'automatic root compatibility starts a window, clears old errors and uses the app HOME':'non-root Electron runs with renderer sandbox enabled, a window and its own HOME');
 const marker=path.join(proc.home,'launch-persistence');writeFileSync(marker,'preserved');
 await stop();await wait('process stopped',async()=>{try{await control();return false;}catch{return true;}});
 await api(`/api/apps/${appId}/start`,'POST');await wait('restart',control);assert.equal(readFileSync(marker,'utf8'),'preserved');pass('restart preserves application data');
 await stop();
 await api(`/api/apps/${appId}/sharing`,'PUT',{enabled:true,password:'launch-fixture-password'});
 await api(`/api/apps/${appId}/start`,'POST');
 const worker=await wait('worker window',control);assert.equal(worker.session,'wayland');
 const workerIdentity=await fetch(`http://127.0.0.1:${controlPort}/identity`).then(r=>r.json());
 assert.equal(workerIdentity.home,proc.home);assert.equal(workerIdentity.cwd,proc.home);assert.equal(workerIdentity.uid,proc.uid);
 assert.equal(workerIdentity.sandboxDisabled,rootUser);
 assert.equal(workerIdentity.ozonePlatform,'wayland');assert.equal(workerIdentity.waylandIme,true);
 const workerInfo=await wait('worker launch details',async()=>{
  const current=(await api(`/api/apps/${appId}/launch-info`)).launch_info;
  return current?.generation!==info.generation&&current?.window_seen?current:null;
 });
 assert.equal(workerInfo.original_command,command);assert.equal(workerInfo.reason,'electron');
 assert(workerInfo.added_args.includes('--enable-wayland-ime'));
 assert.equal((await details()).status,'running');pass('private worker uses the same app identity and HOME with its own display');
 await stop();
 await api(`/api/apps/${appId}/sharing`,'PUT',{enabled:false});
 await update(command+' --ozone-platform=x11');
 const conflict=await api(`/api/apps/${appId}/start`,'POST',undefined,true);assert.equal(conflict.code,'desktop_backend_conflict');
 assert.equal((await details()).launch_error.code,'desktop_backend_conflict');
 await update(working,{compatibility:'auto',sandbox:'auto'});
 const invalid=await api(`/api/apps/${appId}`,'PUT',{desktop_launch:{compatibility:'unknown'}},true);
 assert.equal(invalid.code,'desktop_launch_invalid');assert.equal((await details()).desktop_launch.compatibility,'auto');
 pass('explicit backend conflicts and invalid compatibility settings are rejected without changing saved settings');
 if(rootUser){
  await update(command,{compatibility:'raw',sandbox:'auto'});
  await api(`/api/apps/${appId}/start`,'POST');
  await wait('raw root sandbox diagnostic',async()=>(await details()).launch_error?.code==='desktop_root_sandbox');
  const raw=(await api(`/api/apps/${appId}/launch-info`)).launch_info;
  assert.equal(raw.effective_command,command);assert.equal(raw.reason,'raw');
  await stop();await update(command,{compatibility:'auto',sandbox:'auto'});
  pass('raw execution preserves the command and does not auto-disable the sandbox or retry');
 }

 // Missing display is a real preflight failure, not a shell-command heuristic.
 // Read the display selected by this isolated server, never any host socket.
 const envFile=readFileSync(path.join(env.XDG_RUNTIME_DIR,'ivnc','env'),'utf8');
 const display=envFile.match(/^WAYLAND_DISPLAY=(.*)$/m)?.[1];assert(display);
 rmSync(path.join(env.XDG_RUNTIME_DIR,display));
 const failure=await api(`/api/apps/${appId}/start`,'POST',undefined,true);assert.equal(failure.code,'desktop_display_unavailable');
 assert.equal((await details()).launch_error.code,'desktop_display_unavailable');pass('unavailable display is rejected before spawning and remains visible in app details');
 console.log(`${passed} launch checks passed (${rootUser?'root':'non-root'}).`);
}catch(error){process.exitCode=1;console.error(error.stack);console.error(`Artifacts: ${work}`);}
finally{
 if(appId)await api(`/api/apps/${appId}/stop`,'POST',undefined,true).catch(()=>{});
 server.kill('SIGTERM');await Promise.race([new Promise(r=>server.once('exit',r)),delay(6000)]);
 if(server.exitCode===null&&server.signalCode===null)server.kill('SIGKILL');
 if(!process.exitCode&&!process.env.KEEP_TMP)rmSync(work,{recursive:true,force:true});
}
