#include <wayland-client.h>
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <linux/input-event-codes.h>
#include <time.h>
#include <unistd.h>
#include "pointer.h"
static struct zwlr_virtual_pointer_manager_v1 *manager;
static void global(void *data,struct wl_registry *registry,uint32_t name,const char *interface,uint32_t version) {
 if (!strcmp(interface,"zwlr_virtual_pointer_manager_v1"))
  manager=wl_registry_bind(registry,name,&zwlr_virtual_pointer_manager_v1_interface,1);
}
static void removed(void*d,struct wl_registry*r,uint32_t n){}
static uint32_t timestamp(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return now.tv_sec * 1000 + now.tv_nsec / 1000000;
}

static void submit(struct wl_display *display, struct zwlr_virtual_pointer_v1 *pointer) {
    zwlr_virtual_pointer_v1_frame(pointer);
    wl_display_roundtrip(display);
    usleep(60000);
}

// Width and height are the single headless output's logical dimensions.
static void drag(struct wl_display *display, struct zwlr_virtual_pointer_v1 *pointer,
                 int from, int to, int y, int width, int height) {
    zwlr_virtual_pointer_v1_motion_absolute(pointer, timestamp(), from, y, width, height);
    submit(display, pointer);
    zwlr_virtual_pointer_v1_button(pointer, timestamp(), BTN_LEFT, WL_POINTER_BUTTON_STATE_PRESSED);
    submit(display, pointer);
    for (int step = 1; step <= 8; ++step) {
        int x = from + (to - from) * step / 8;
        zwlr_virtual_pointer_v1_motion_absolute(pointer, timestamp(), x, y, width, height);
        submit(display, pointer);
    }
    zwlr_virtual_pointer_v1_button(pointer, timestamp(), BTN_LEFT, WL_POINTER_BUTTON_STATE_RELEASED);
    submit(display, pointer);
}

int main(int argc, char **argv) {
 struct wl_display*d=wl_display_connect(NULL);
 if(!d)return 1;
 struct wl_registry*r=wl_display_get_registry(d);
 static const struct wl_registry_listener listener={global,removed};
 wl_registry_add_listener(r,&listener,NULL);wl_display_roundtrip(d);
 if(!manager)return 2;
 struct zwlr_virtual_pointer_v1 *pointer = zwlr_virtual_pointer_manager_v1_create_virtual_pointer(manager,NULL);
 wl_display_roundtrip(d);
 if(argc == 7 && !strcmp(argv[1], "drag")) {
     drag(d, pointer, atoi(argv[2]), atoi(argv[3]), atoi(argv[4]), atoi(argv[5]), atoi(argv[6]));
     wl_display_disconnect(d);
     return 0;
 }
 puts("ready");fflush(stdout);
 while(wl_display_dispatch(d)>=0){}
 return 0;
}
