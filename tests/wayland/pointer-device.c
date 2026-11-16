#include <wayland-client.h>
#include <string.h>
#include <stdio.h>
#include <unistd.h>
#include "pointer.h"
static struct zwlr_virtual_pointer_manager_v1 *manager;
static void global(void *data,struct wl_registry *registry,uint32_t name,const char *interface,uint32_t version) {
 if (!strcmp(interface,"zwlr_virtual_pointer_manager_v1"))
  manager=wl_registry_bind(registry,name,&zwlr_virtual_pointer_manager_v1_interface,1);
}
static void removed(void*d,struct wl_registry*r,uint32_t n){}
int main(void) {
 struct wl_display*d=wl_display_connect(NULL);
 if(!d)return 1;
 struct wl_registry*r=wl_display_get_registry(d);
 static const struct wl_registry_listener listener={global,removed};
 wl_registry_add_listener(r,&listener,NULL);wl_display_roundtrip(d);
 if(!manager)return 2;
 zwlr_virtual_pointer_manager_v1_create_virtual_pointer(manager,NULL);
 wl_display_roundtrip(d);
 puts("ready");fflush(stdout);
 while(wl_display_dispatch(d)>=0){}
 return 0;
}
