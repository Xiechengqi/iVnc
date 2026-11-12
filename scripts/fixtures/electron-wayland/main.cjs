// Test-only Electron app. The loopback control endpoint exists only in this
// fixture, never in iVnc. ELECTRON_BIN selects the runtime in the smoke script.
const { app, BrowserWindow, Menu, clipboard, nativeImage, dialog, ClipboardItem } = require('electron');
const http = require('node:http');
const fs = require('node:fs');
const windows = new Map(), events = [];
let popupMenu, menuOpen = false, menuShows = 0, menuCloses = 0;
const page = name => 'data:text/html;charset=utf-8,' + encodeURIComponent(`<!doctype html>
<title>${name}</title><style>body{margin:0;background:#22314b;color:#fff;font:18px sans-serif}textarea{display:block;width:90%;height:160px;margin:15px;font:20px sans-serif}#drag{height:32px;-webkit-app-region:drag;background:#4b6483}#custom{width:100px;height:50px}#lock{width:160px;height:40px}#edit{margin:15px;min-height:70px;border:1px solid white}</style>
<div id="drag">${name} — drag here</div><textarea id="text" autofocus></textarea><div contenteditable id="edit"></div><input id="file" type="file"><button id="lock" onclick="this.requestPointerLock()">Lock pointer</button><div id="custom">Custom cursor</div>
<script>
window.pastes=[];window.moves=[];window.inputs=[];window.clicks=0;document.addEventListener('click',()=>clicks++);
document.addEventListener('paste',e=>pastes.push({text:e.clipboardData.getData('text/plain'),html:e.clipboardData.getData('text/html'),types:[...e.clipboardData.types]}));
document.addEventListener('mousemove',e=>moves.push([e.clientX,e.clientY,e.movementX,e.movementY]));
document.addEventListener('input',e=>inputs.push(e.target.value));
const canvas=document.createElement('canvas');canvas.width=canvas.height=24;const c=canvas.getContext('2d');c.fillStyle='#e64c80';c.fillRect(0,0,18,18);c.fillStyle='#ffffff';c.fillRect(4,4,10,10);document.getElementById('custom').style.cursor='url('+canvas.toDataURL()+') 3 4, auto';
</script>`);
async function create(name, options = {}) {
    const window = new BrowserWindow({title:name,width:620,height:460,show:false,autoHideMenuBar:true,...options});
    windows.set(name, window);
    for (const event of ['focus','blur','minimize','restore','maximize','unmaximize','enter-full-screen','leave-full-screen','move','resize','closed']) {
        window.on(event, () => { events.push([name,event]); if (event==='closed') windows.delete(name); });
    }
    await window.loadURL(page(name));
    window.show(); return window;
}
function state() {
    return {version:process.versions.electron,session:process.env.XDG_SESSION_TYPE,display:process.env.DISPLAY||null,
        windows:[...windows].map(([name,w])=>({name,id:w.id,bounds:w.getBounds(),maximized:w.isMaximized(),fullscreen:w.isFullScreen(),minimized:w.isMinimized(),focused:w.isFocused()})),events:events.slice(-100),
        menu:{open:menuOpen,shows:menuShows,closes:menuCloses}};
}
app.whenReady().then(async () => {
    await create('Electron Main');
    const server = http.createServer(async (req,res) => {
        try {
            const url=new URL(req.url,'http://localhost'), action=url.pathname.slice(1), name=url.searchParams.get('name')||'Electron Main';
            const w=windows.get(name); let result;
            switch(action) {
                case 'state': result=state(); break;
                case 'second': await create('Electron Second'); result=state(); break;
                case 'dialog': await create('Electron Dialog',{parent:w,modal:true,resizable:false,width:340,height:230}); result=state(); break;
                case 'fixed': await create('Electron Fixed',{resizable:false,width:360,height:240}); result=state(); break;
                case 'minimize': w.minimize(); result=true; break;
                case 'restore': w.restore(); result=true; break;
                case 'maximize': w.maximize(); result=true; break;
                case 'unmaximize': w.unmaximize(); result=true; break;
                case 'bounds': w.setBounds({x:0,y:0,width:680,height:420}); result=true; break;
                case 'fullscreen': w.setFullScreen(url.searchParams.get('on')==='1'); result=true; break;
                case 'focus': w.focus(); result=true; break;
                case 'close': w.close(); result=true; break;
                case 'eval': result=await w.webContents.executeJavaScript(url.searchParams.get('js')); break;
                case 'menu':
                    popupMenu = Menu.buildFromTemplate([{label:'Item one'},{label:'Item two'}]);
                    popupMenu.on('menu-will-show',()=>{menuOpen=true;menuShows++});
                    popupMenu.popup({window:w,x:300,y:300,callback:()=>{menuOpen=false;menuCloses++}});
                    result=true; break;
                case 'clipboard': {
                    const image=nativeImage.createFromBitmap(Buffer.from([0,0,255,255]),{width:1,height:1});
                    if (ClipboardItem) await clipboard.write([new ClipboardItem({'text/plain':'Electron text','text/html':'<b>Electron rich text</b>','image/png':new Blob([image.toPNG()],{type:'image/png'})})]);
                    else clipboard.write({text:'Electron text',html:'<b>Electron rich text</b>',image});
                    result=true; break;
                }
                case 'readClipboard': {
                    if (ClipboardItem) {
                        const items=await clipboard.read(), formats=items.flatMap(i=>i.types);
                        const htmlItem=items.find(i=>i.types.includes('text/html'));
                        result={text:await clipboard.readText(),html:htmlItem?await (await htmlItem.getType('text/html')).text():'',formats,image:formats.includes('image/png')};
                    } else result={text:clipboard.readText(),html:clipboard.readHTML(),formats:clipboard.availableFormats(),image:!clipboard.readImage().isEmpty()};
                    break;
                }
                case 'file': result=fs.readFileSync(url.searchParams.get('path'),'utf8'); break;
                case 'nativeFile': result=await dialog.showOpenDialog(w,{title:'Electron File Picker',properties:['openFile'],defaultPath:process.env.HOME}); break;
                default: throw Error('Unknown fixture action');
            }
            res.setHeader('Content-Type','application/json'); res.end(JSON.stringify(result ?? null));
        } catch(e) { res.statusCode=500;res.end(JSON.stringify({error:String(e)})); }
    });
    server.listen(Number(process.env.ELECTRON_CONTROL_PORT),'127.0.0.1');
});
app.on('window-all-closed',()=>app.quit());
