// Isolated process-tree and diagnostic tests for the single compatibility
// retry. A small native fixture simulates fatal errors before any GUI exists.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, openSync, closeSync, rmSync } from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';

const root=path.resolve(new URL('..',import.meta.url).pathname);
const binary=path.resolve(process.env.IVNC_BIN||path.join(root,'target/debug/ivnc'));
const work=mkdtempSync(path.join(tmpdir(),'ivnc-compat-smoke-'));
const env={...process.env,HOME:path.join(work,'home'),IVNC_HOME:path.join(work,'ivnc'),XDG_RUNTIME_DIR:path.join(work,'run')};
for(const key of ['HOME','IVNC_HOME','XDG_RUNTIME_DIR'])mkdirSync(env[key],{mode:0o700});
const source=path.join(work,'fixture.c');
writeFileSync(source,`#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
static void trace(const char *s) { int f=open(getenv("FIXTURE_TRACE"),O_WRONLY|O_CREAT|O_APPEND,0600); dprintf(f,"%s\\n",s);close(f); }
int main(int argc,char **argv) {
#if ELECTRON_EVIDENCE
 const char *evidence="Electron/44.9.3 ELECTRON_RUN_AS_NODE ELECTRON_NO_ASAR ozone-platform enable-wayland-ime wayland-text-input-version";
 if(getenv("FIXTURE_PRINT_EVIDENCE"))puts(evidence);
#endif
 int disabled=0; for(int i=1;i<argc;i++) if(!strcmp(argv[i],"--no-sandbox"))disabled=1;
 trace(disabled?"attempt:adapted":"attempt:original");
 if(!disabled) {
  if(getenv("FIXTURE_GENERIC_X11")) { fprintf(stderr,"Missing X server or $DISPLAY\\n"); return 1; }
  fprintf(stderr,"[FATAL:electron/shell/app/electron_main_delegate.cc:227] Running as root without --no-sandbox is not supported\\n");
  if(!fork()){ usleep(1500000);trace("first-child-finished");return 0; }
  return 1;
 }
 if(getenv("FIXTURE_FAIL_SECOND")) { fprintf(stderr,"No usable sandbox!\\n");return 1; }
 while(1)pause();
}
`);
const executable=path.join(work,'opaque-electron'),launcher=path.join(work,'generic-launcher');
execFileSync('cc',['-DELECTRON_EVIDENCE=1',source,'-o',executable]);
execFileSync('cc',['-DELECTRON_EVIDENCE=0',source,'-o',launcher]);
const socket=net.createServer();await new Promise(r=>socket.listen(0,'127.0.0.1',r));const port=socket.address().port;await new Promise(r=>socket.close(r));
const config=path.join(work,'config.toml');writeFileSync(config,'[audio]\nenabled=false\nsample_rate=48000\nchannels=2\nbitrate=128000\n');
const fd=openSync(path.join(work,'server.log'),'a',0o600);
const server=spawn(binary,['--config',config,'--http-port',String(port),'--basic-auth-enabled','false','--proxy-enabled','false','--proxy-autostart','false'],{env,cwd:work,stdio:['ignore',fd,fd]});closeSync(fd);
const base=`http://127.0.0.1:${port}`,apps=[];
let passed=0;
const pass=name=>{passed++;console.log(`PASS ${name}`)};
async function wait(name,fn,timeout=20000){const end=Date.now()+timeout;while(Date.now()<end){try{const value=await fn();if(value)return value;}catch{}await delay(80);}throw Error(`Timeout: ${name}`);}
async function api(url,method='GET',body){const response=await fetch(base+url,{method,headers:{'Content-Type':'application/json'},body:body?JSON.stringify(body):undefined});const value=await response.json();assert(response.ok,JSON.stringify(value));return value;}
const quote=s=>"'"+s.replaceAll("'","'\\''")+"'";
const lines=file=>{try{return readFileSync(file,'utf8').trim().split('\n');}catch{return [];}};
async function create(name,extra={},program=executable){
 const trace=path.join(work,name+'.trace');
 const result=await api('/api/apps','POST',{name,app_type:'desktop',exec_command:quote(program),env_vars:{FIXTURE_TRACE:trace,...extra}});
 const id=result.app.id;apps.push(id);await api(`/api/apps/${id}/start`,'POST');return {id,trace};
}
try{
 await wait('server',()=>api('/api/apps'));
 if(process.geteuid()!==0)throw Error('Run this fixture as root; non-root sandbox behavior is covered by launch_smoke.mjs.');
 const success=await create('one-retry');
 await wait('corrected attempt',()=>lines(success.trace).includes('attempt:adapted'));
 assert.deepEqual(lines(success.trace),['attempt:original','first-child-finished','attempt:adapted']);
 const info=(await api(`/api/apps/${success.id}/launch-info`)).launch_info;
 assert.equal(info.reason,'electron_retry');assert.equal(info.retried,true);assert.equal(info.original_command,quote(executable));
 assert.equal((await api(`/api/apps/${success.id}`)).launch_error,null);
 await api(`/api/apps/${success.id}/stop`,'POST');
 pass('retry waits for every descendant of the first attempt and preserves the original command');
 await api(`/api/apps/${success.id}/sharing`,'PUT',{enabled:true,password:'compat-worker-fixture'});
 writeFileSync(success.trace,'');await api(`/api/apps/${success.id}/start`,'POST');
 await wait('worker corrected attempt',()=>lines(success.trace).includes('attempt:adapted'));
 assert.deepEqual(lines(success.trace),['attempt:original','first-child-finished','attempt:adapted']);
 const workerInfo=(await api(`/api/apps/${success.id}/launch-info`)).launch_info;
 assert.notEqual(workerInfo.generation,info.generation);assert.equal(workerInfo.reason,'electron_retry');
 assert.equal((await api(`/api/apps/${success.id}`)).launch_error,null);
 await api(`/api/apps/${success.id}/stop`,'POST');
 pass('private workers diagnose and report the corrected attempt in the same durable app directory');


 const stopped=await create('stop-before-retry');
 await wait('first attempt',()=>lines(stopped.trace).includes('attempt:original'));
 await api(`/api/apps/${stopped.id}/stop`,'POST');await delay(2000);
 assert(!lines(stopped.trace).includes('attempt:adapted'));
 assert.equal((await api(`/api/apps/${stopped.id}`)).status,'stopped');
 pass('a user stop cancels compatibility retry and ends the process tree');

 const failed=await create('second-failure',{FIXTURE_FAIL_SECOND:'1'});
 await wait('second failure',async()=>(await api(`/api/apps/${failed.id}`)).launch_error?.code==='desktop_sandbox_unavailable');
 await delay(6500);
 assert.equal(lines(failed.trace).filter(l=>l.startsWith('attempt:')).length,2);
 pass('a failed corrected attempt reports its own error and cannot trigger a third launch');

 const opaque=await create('opaque-launcher',{},launcher);
 await wait('opaque launcher exit',async()=>(await api(`/api/apps/${opaque.id}`)).launch_error?.code==='desktop_root_sandbox');
 assert(!lines(opaque.trace).includes('attempt:adapted'));
 pass('Electron errors inside an unconfirmed native launcher cannot add arguments to that launcher');

 const x11=await create('generic-x11',{FIXTURE_GENERIC_X11:'1'});
 await wait('generic X11 exit',async()=>(await api(`/api/apps/${x11.id}`)).launch_error?.code==='desktop_x11_backend');
 assert.deepEqual(lines(x11.trace),['attempt:original']);
 pass('generic X11 errors do not identify an Electron application');
 console.log(`${passed} compatibility retry checks passed.`);
}catch(error){process.exitCode=1;console.error(error.stack);console.error(`Artifacts: ${work}`);}
finally{
 for(const id of apps)await api(`/api/apps/${id}/stop`,'POST').catch(()=>{});
 server.kill('SIGTERM');await Promise.race([new Promise(r=>server.once('exit',r)),delay(6000)]);
 if(server.exitCode===null&&server.signalCode===null)server.kill('SIGKILL');
 if(!process.exitCode&&!process.env.KEEP_TMP)rmSync(work,{recursive:true,force:true});
}
