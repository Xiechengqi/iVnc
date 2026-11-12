// Real Electron launch diagnostics, recovery and same-UID sessions. Run as root
// and again as a non-root user with a working Electron sandbox.
// ELECTRON_BIN=/path/to/electron IVNC_BIN=target/debug/ivnc node scripts/launch_smoke.mjs
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, openSync, closeSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
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
const fd=openSync(path.join(work,'server.log'),'a',0o600);
const server=spawn(binary,['--config',config,'--http-port',String(httpPort),'--basic-auth-enabled','false','--proxy-enabled','false','--proxy-autostart','false','--width','800','--height','600'],{env,cwd:work,stdio:['ignore',fd,fd]});closeSync(fd);
let appId,passed=0;
const pass=s=>{passed++;console.log(`PASS ${s}`)};
const quote=s=>"'"+s.replaceAll("'","'\\''")+"'";
const command=[electron,path.join(root,'scripts/fixtures/electron-wayland/main.cjs')].map(quote).join(' ')+' --ozone-platform=wayland';
const rootUser=process.geteuid()===0;
const working=command+(rootUser?' --no-sandbox':'');
async function api(url,method='GET',body,allowError=false){
 const r=await fetch(base+url,{method,headers:{'Content-Type':'application/json'},body:body?JSON.stringify(body):undefined});
 const d=await r.json();if(!allowError)assert(r.ok,JSON.stringify(d));return d;
}
async function wait(name,fn,ms=25000){const end=Date.now()+ms;let last;while(Date.now()<end){try{const v=await fn();if(v)return v;}catch(e){last=e;}await delay(100);}throw Error(`${name}: ${last||'timed out'}`);}
const details=()=>api(`/api/apps/${appId}`);
const control=()=>fetch(`http://127.0.0.1:${controlPort}/state`).then(r=>r.json());
async function update(exec){await api(`/api/apps/${appId}`,'PUT',{exec_command:exec,env_vars:{ELECTRON_CONTROL_PORT:String(controlPort)}});}
async function stop(){await api(`/api/apps/${appId}/stop`,'POST');}
try{
 await wait('server',()=>api('/api/apps/launch-context'));
 const context=await api('/api/apps/launch-context');assert.equal(context.identity.uid,process.geteuid());assert.equal(context.identity.gid,process.getegid());pass('launch context reports actual UID/GID');
 const created=await api('/api/apps','POST',{name:'Launch diagnostic fixture',app_type:'desktop',exec_command:rootUser?command:working,env_vars:{ELECTRON_CONTROL_PORT:String(controlPort)}});appId=created.app.id;
 if(rootUser){
  await api(`/api/apps/${appId}/start`,'POST');
  await wait('root sandbox diagnostic',async()=>(await details()).launch_error?.code==='desktop_root_sandbox');
  await delay(8000);
  const log=readFileSync(path.join(env.IVNC_HOME,'apps',appId,'app.log'),'utf8');
  assert.equal(log.split('Running as root without --no-sandbox is not supported').length-1,1);pass('real Electron root sandbox failure is explained and not automatically retried');
  await update(working);
 }
 await api(`/api/apps/${appId}/start`,'POST');
 const state=await wait('Electron window',control);assert.equal(state.session,'wayland');
 await wait('window status',async()=>(await details()).window_ready===true);
 assert.equal((await details()).launch_error,null);
 const proc=await fetch(`http://127.0.0.1:${controlPort}/identity`).then(r=>r.json());
 assert.equal(proc.uid,process.geteuid());assert.equal(proc.cwd,proc.home);
 assert.equal(proc.sandboxDisabled,rootUser);
 if(!rootUser){
  assert.equal(proc.rendererSandboxPreferred,true);
  const status=readFileSync(`/proc/${proc.rendererPid}/status`,'utf8');
  assert.match(status,/^NoNewPrivs:\s+1$/m);assert.match(status,/^Seccomp:\s+2$/m);
 }
 pass(rootUser?'explicit root compatibility starts a window, clears old errors and uses the app HOME':'non-root Electron runs with renderer sandbox enabled, a window and its own HOME');
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
 assert.equal((await details()).status,'running');pass('private worker uses the same app identity and HOME with its own display');
 await stop();
 await api(`/api/apps/${appId}/sharing`,'PUT',{enabled:false});
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
