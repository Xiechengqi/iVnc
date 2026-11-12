// Minimal protocol client for parent/identity/drag regressions that Electron
// cannot express on Wayland. Built by electron_smoke.mjs using wayland-scanner.
#define _GNU_SOURCE
#include <wayland-client.h>
#include "xdg-shell-client-protocol.h"
#include <sys/mman.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <poll.h>
#include <unistd.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <stdint.h>
#include <errno.h>

struct window {struct wl_surface *surface; struct xdg_surface *xdg; struct xdg_toplevel *top; int width,height,focused,pressed;};
static struct wl_display *display;
static struct wl_compositor *compositor;
static struct wl_shm *shm;
static struct xdg_wm_base *shell;
static struct wl_seat *seat;
static struct wl_pointer *pointer;
static struct window windows[2];
static int count=2, pointer_window=-1;
static double pointer_y;

struct buffer {void *memory; size_t size;};
static void release(void *data,struct wl_buffer *b){struct buffer *x=data;wl_buffer_destroy(b);munmap(x->memory,x->size);free(x);}
static const struct wl_buffer_listener buffer_listener={release};
static void repaint(struct window *w){
    struct buffer *data=calloc(1,sizeof(*data));
    data->size=(size_t)w->width*w->height*4;
    int fd=memfd_create("ivnc-test",MFD_CLOEXEC);if(fd<0||ftruncate(fd,data->size))exit(2);
    data->memory=mmap(NULL,data->size,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0);if(data->memory==MAP_FAILED)exit(2);
    uint32_t *pixels=data->memory;
    for(size_t i=0;i<data->size/4;i++)pixels[i]=w==windows?0xff226688:0xffbb4433;
    struct wl_shm_pool *pool=wl_shm_create_pool(shm,fd,data->size);
    struct wl_buffer *buffer=wl_shm_pool_create_buffer(pool,0,w->width,w->height,w->width*4,WL_SHM_FORMAT_XRGB8888);
    wl_buffer_add_listener(buffer,&buffer_listener,data);wl_shm_pool_destroy(pool);close(fd);
    wl_surface_attach(w->surface,buffer,0,0);wl_surface_damage(w->surface,0,0,w->width,w->height);wl_surface_commit(w->surface);
}
static void configured(void *data,struct xdg_surface *s,uint32_t serial){xdg_surface_ack_configure(s,serial);repaint(data);}
static const struct xdg_surface_listener surface_listener={configured};
static void configure(void *data,struct xdg_toplevel *t,int32_t w,int32_t h,struct wl_array *states){
    struct window *window=data;if(w>0)window->width=w;if(h>0)window->height=h;window->focused=0;
    uint32_t *s;wl_array_for_each(s,states)if(*s==XDG_TOPLEVEL_STATE_ACTIVATED)window->focused=1;
}
static void close_window(void *data,struct xdg_toplevel *t){exit(0);}
static const struct xdg_toplevel_listener top_listener={.configure=configure,.close=close_window};
static void ping(void *data,struct xdg_wm_base *s,uint32_t serial){xdg_wm_base_pong(s,serial);}
static const struct xdg_wm_base_listener shell_listener={ping};
static void pointer_enter(void *data,struct wl_pointer *p,uint32_t serial,struct wl_surface *surface,wl_fixed_t x,wl_fixed_t y){
    pointer_window=-1;for(int i=0;i<count;i++)if(windows[i].surface==surface)pointer_window=i;pointer_y=wl_fixed_to_double(y);
}
static void pointer_leave(void *data,struct wl_pointer *p,uint32_t serial,struct wl_surface *surface){pointer_window=-1;}
static void pointer_motion(void *data,struct wl_pointer *p,uint32_t time,wl_fixed_t x,wl_fixed_t y){pointer_y=wl_fixed_to_double(y);}
static void pointer_button(void *data,struct wl_pointer *p,uint32_t serial,uint32_t time,uint32_t button,uint32_t state){
    if(pointer_window>=0&&state==WL_POINTER_BUTTON_STATE_PRESSED){struct window *w=windows+pointer_window;w->pressed++;
        if(pointer_window==1&&pointer_y<30)xdg_toplevel_move(w->top,seat,serial);
    }
}
static void pointer_axis(void *data,struct wl_pointer *p,uint32_t time,uint32_t axis,wl_fixed_t value){}
static const struct wl_pointer_listener pointer_listener={.enter=pointer_enter,.leave=pointer_leave,.motion=pointer_motion,.button=pointer_button,.axis=pointer_axis};
static void capabilities(void *data,struct wl_seat *s,uint32_t caps){if((caps&WL_SEAT_CAPABILITY_POINTER)&&!pointer){pointer=wl_seat_get_pointer(s);wl_pointer_add_listener(pointer,&pointer_listener,NULL);}}
static void seat_name(void *data,struct wl_seat *s,const char *name){}
static const struct wl_seat_listener seat_listener={capabilities,seat_name};
static void global(void *data,struct wl_registry *r,uint32_t name,const char *interface,uint32_t version){
    if(!strcmp(interface,"wl_compositor"))compositor=wl_registry_bind(r,name,&wl_compositor_interface,3);
    else if(!strcmp(interface,"wl_shm"))shm=wl_registry_bind(r,name,&wl_shm_interface,1);
    else if(!strcmp(interface,"xdg_wm_base")){shell=wl_registry_bind(r,name,&xdg_wm_base_interface,1);xdg_wm_base_add_listener(shell,&shell_listener,NULL);}
    else if(!strcmp(interface,"wl_seat")){seat=wl_registry_bind(r,name,&wl_seat_interface,1);wl_seat_add_listener(seat,&seat_listener,NULL);}
}
static void removed(void *data,struct wl_registry *r,uint32_t name){}
static const struct wl_registry_listener registry_listener={global,removed};
static void serve(int server){
    int c=accept4(server,NULL,NULL,SOCK_CLOEXEC);if(c<0)return;char request[4096]={0};recv(c,request,sizeof(request)-1,0);
    if(strstr(request,"GET /minimize "))xdg_toplevel_set_minimized(windows[0].top);
    if(strstr(request,"GET /repaint "))repaint(windows+1);
    if(strstr(request,"GET /unparent "))xdg_toplevel_set_parent(windows[1].top,NULL);
    if(strstr(request,"GET /reparent "))xdg_toplevel_set_parent(windows[1].top,windows[0].top);
    if(strstr(request,"GET /destroyParent ")){
        xdg_toplevel_destroy(windows[0].top);xdg_surface_destroy(windows[0].xdg);wl_surface_destroy(windows[0].surface);
        windows[0].top=NULL;windows[0].xdg=NULL;windows[0].surface=NULL;
    }
    char body[1024],response[1300];
    snprintf(body,sizeof(body),"{\"rootId\":%u,\"childId\":%u,\"rootFocused\":%d,\"childFocused\":%d,\"childPressed\":%d,\"width\":%d,\"height\":%d}",windows[0].surface?wl_proxy_get_id((struct wl_proxy*)windows[0].surface):0,count>1?wl_proxy_get_id((struct wl_proxy*)windows[1].surface):0,windows[0].focused,windows[1].focused,windows[1].pressed,windows[0].width,windows[0].height);
    snprintf(response,sizeof(response),"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: %zu\r\nConnection: close\r\n\r\n%s",strlen(body),body);send(c,response,strlen(response),MSG_NOSIGNAL);close(c);
}
int main(void){
    display=wl_display_connect(NULL);if(!display)return 2;
    struct wl_registry *registry=wl_display_get_registry(display);wl_registry_add_listener(registry,&registry_listener,NULL);
    wl_display_roundtrip(display);wl_display_roundtrip(display);if(!compositor||!shm||!shell||!seat)return 2;
    if(getenv("FIXTURE_SINGLE"))count=1;
    unsigned target=getenv("FIXTURE_SURFACE_ID")?atoi(getenv("FIXTURE_SURFACE_ID")):0;
    const char *name=getenv("FIXTURE_NAME")?getenv("FIXTURE_NAME"):"Protocol Parent";
    for(int i=0;i<count;i++){
        struct window *w=windows+i;w->width=i?320:640;w->height=i?180:480;
        w->surface=wl_compositor_create_surface(compositor);
        if(i==0&&target){
            while(wl_proxy_get_id((struct wl_proxy*)w->surface)<target)w->surface=wl_compositor_create_surface(compositor);
            if(wl_proxy_get_id((struct wl_proxy*)w->surface)!=target)return 2;
        }
        w->xdg=xdg_wm_base_get_xdg_surface(shell,w->surface);
        xdg_surface_add_listener(w->xdg,&surface_listener,w);w->top=xdg_surface_get_toplevel(w->xdg);
        xdg_toplevel_add_listener(w->top,&top_listener,w);xdg_toplevel_set_title(w->top,i?"Protocol Child":name);xdg_toplevel_set_app_id(w->top,"ivnc-wayland-fixture");
        if(i)xdg_toplevel_set_parent(w->top,windows[0].top);
        wl_surface_commit(w->surface);
    }
    int server=socket(AF_INET,SOCK_STREAM|SOCK_CLOEXEC,0),one=1;setsockopt(server,SOL_SOCKET,SO_REUSEADDR,&one,sizeof(one));
    struct sockaddr_in address={.sin_family=AF_INET,.sin_addr.s_addr=htonl(INADDR_LOOPBACK),.sin_port=htons(atoi(getenv("FIXTURE_PORT")))};
    if(bind(server,(struct sockaddr*)&address,sizeof(address))||listen(server,4))return 2;
    for(;;){
        if(wl_display_dispatch_pending(display)<0)return 1;
        wl_display_flush(display);
        struct pollfd fds[2]={{wl_display_get_fd(display),POLLIN,0},{server,POLLIN,0}};
        if(poll(fds,2,-1)<0){if(errno==EINTR)continue;return 1;}
        if(fds[0].revents&POLLIN)if(wl_display_dispatch(display)<0)return 1;
        if(fds[0].revents&(POLLHUP|POLLERR))return 1;
        if(fds[1].revents&POLLIN)serve(server);
    }
}
