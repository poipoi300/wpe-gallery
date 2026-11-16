// Test-only Vulkan layer: emulates NVIDIA's Wayland WSI on niri, where the
// first configured swapchain starts returning VK_ERROR_OUT_OF_DATE_KHR from
// vkAcquireNextImageKHR after its first frames and stays unusable until the
// application recreates it. Only the first swapchain that reaches
// WPE_TEST_OUT_OF_DATE_AFTER successful acquires (default 2) is poisoned;
// recreated swapchains behave normally. Loaded explicitly via
// VK_LAYER_PATH + VK_INSTANCE_LAYERS by tests/gui_startup.py only.
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <vulkan/vulkan.h>
#include <vulkan/vk_layer.h>

#define EXPORT __attribute__((visibility("default")))

static PFN_vkGetInstanceProcAddr next_gipa;
static PFN_vkGetDeviceProcAddr next_gdpa;
static PFN_vkAcquireNextImageKHR next_acquire;
static PFN_vkCreateSwapchainKHR next_create_swapchain;
static PFN_vkDestroyDevice next_destroy_device;

static VkSwapchainKHR current = VK_NULL_HANDLE, poisoned = VK_NULL_HANDLE;
static int acquires_on_current, poison_used, limit = -1;
static unsigned long rejected, created;

static void report(void) {
    fprintf(stderr, "[out-of-date-layer] swapchains=%lu rejected_acquires=%lu\n", created, rejected);
}

static VKAPI_ATTR VkResult VKAPI_CALL layer_create_swapchain(VkDevice device, const VkSwapchainCreateInfoKHR *info,
                                                             const VkAllocationCallbacks *alloc, VkSwapchainKHR *out) {
    VkResult r = next_create_swapchain(device, info, alloc, out);
    if (r == VK_SUCCESS) { current = *out; acquires_on_current = 0; created++; }
    return r;
}

static VKAPI_ATTR VkResult VKAPI_CALL layer_acquire(VkDevice device, VkSwapchainKHR swapchain, uint64_t timeout,
                                                    VkSemaphore semaphore, VkFence fence, uint32_t *index) {
    if (limit < 0) { const char *e = getenv("WPE_TEST_OUT_OF_DATE_AFTER"); limit = e ? atoi(e) : 2; }
    if (swapchain == poisoned) {
        if (rejected++ % 100 == 0) fprintf(stderr, "[out-of-date-layer] rejected acquire #%lu\n", rejected);
        return VK_ERROR_OUT_OF_DATE_KHR;
    }
    VkResult r = next_acquire(device, swapchain, timeout, semaphore, fence, index);
    if ((r == VK_SUCCESS || r == VK_SUBOPTIMAL_KHR) && swapchain == current && !poison_used
        && ++acquires_on_current >= limit) {
        poisoned = swapchain; poison_used = 1;
        fprintf(stderr, "[out-of-date-layer] poisoned swapchain after %d acquires\n", acquires_on_current);
    }
    return r;
}

static VKAPI_ATTR void VKAPI_CALL layer_destroy_device(VkDevice device, const VkAllocationCallbacks *alloc) {
    report();
    next_destroy_device(device, alloc);
}

static VKAPI_ATTR VkResult VKAPI_CALL layer_create_device(VkPhysicalDevice gpu, const VkDeviceCreateInfo *info,
                                                          const VkAllocationCallbacks *alloc, VkDevice *out) {
    VkLayerDeviceCreateInfo *chain = (VkLayerDeviceCreateInfo *)info->pNext;
    while (chain && !(chain->sType == VK_STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO && chain->function == VK_LAYER_LINK_INFO))
        chain = (VkLayerDeviceCreateInfo *)chain->pNext;
    if (!chain) return VK_ERROR_INITIALIZATION_FAILED;
    PFN_vkGetInstanceProcAddr gipa = chain->u.pLayerInfo->pfnNextGetInstanceProcAddr;
    next_gdpa = chain->u.pLayerInfo->pfnNextGetDeviceProcAddr;
    chain->u.pLayerInfo = chain->u.pLayerInfo->pNext;
    PFN_vkCreateDevice create = (PFN_vkCreateDevice)gipa(VK_NULL_HANDLE, "vkCreateDevice");
    VkResult r = create(gpu, info, alloc, out);
    if (r != VK_SUCCESS) return r;
    next_acquire = (PFN_vkAcquireNextImageKHR)next_gdpa(*out, "vkAcquireNextImageKHR");
    next_create_swapchain = (PFN_vkCreateSwapchainKHR)next_gdpa(*out, "vkCreateSwapchainKHR");
    next_destroy_device = (PFN_vkDestroyDevice)next_gdpa(*out, "vkDestroyDevice");
    return VK_SUCCESS;
}

static VKAPI_ATTR VkResult VKAPI_CALL layer_create_instance(const VkInstanceCreateInfo *info,
                                                            const VkAllocationCallbacks *alloc, VkInstance *out) {
    VkLayerInstanceCreateInfo *chain = (VkLayerInstanceCreateInfo *)info->pNext;
    while (chain && !(chain->sType == VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO && chain->function == VK_LAYER_LINK_INFO))
        chain = (VkLayerInstanceCreateInfo *)chain->pNext;
    if (!chain) return VK_ERROR_INITIALIZATION_FAILED;
    next_gipa = chain->u.pLayerInfo->pfnNextGetInstanceProcAddr;
    chain->u.pLayerInfo = chain->u.pLayerInfo->pNext;
    PFN_vkCreateInstance create = (PFN_vkCreateInstance)next_gipa(VK_NULL_HANDLE, "vkCreateInstance");
    return create(info, alloc, out);
}

EXPORT VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL wpe_test_GetDeviceProcAddr(VkDevice device, const char *name) {
    if (!strcmp(name, "vkGetDeviceProcAddr")) return (PFN_vkVoidFunction)wpe_test_GetDeviceProcAddr;
    if (!strcmp(name, "vkAcquireNextImageKHR")) return (PFN_vkVoidFunction)layer_acquire;
    if (!strcmp(name, "vkCreateSwapchainKHR")) return (PFN_vkVoidFunction)layer_create_swapchain;
    if (!strcmp(name, "vkDestroyDevice")) return (PFN_vkVoidFunction)layer_destroy_device;
    return next_gdpa(device, name);
}

EXPORT VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL wpe_test_GetInstanceProcAddr(VkInstance instance, const char *name) {
    if (!strcmp(name, "vkGetInstanceProcAddr")) return (PFN_vkVoidFunction)wpe_test_GetInstanceProcAddr;
    if (!strcmp(name, "vkCreateInstance")) return (PFN_vkVoidFunction)layer_create_instance;
    if (!strcmp(name, "vkCreateDevice")) return (PFN_vkVoidFunction)layer_create_device;
    if (!strcmp(name, "vkGetDeviceProcAddr")) return (PFN_vkVoidFunction)wpe_test_GetDeviceProcAddr;
    if (!strcmp(name, "vkAcquireNextImageKHR")) return (PFN_vkVoidFunction)layer_acquire;
    if (!strcmp(name, "vkCreateSwapchainKHR")) return (PFN_vkVoidFunction)layer_create_swapchain;
    if (!strcmp(name, "vkDestroyDevice")) return (PFN_vkVoidFunction)layer_destroy_device;
    return next_gipa(instance, name);
}
