use ash::vk;
use std::mem;

impl super::Surface {
    pub fn info(&self) -> crate::SurfaceInfo {
        crate::SurfaceInfo {
            format: self.swapchain.format,
            alpha: self.swapchain.alpha,
        }
    }

    unsafe fn deinit_swapchain(&mut self, raw_device: &ash::Device) {
        let _ = raw_device.device_wait_idle();
        self.device
            .destroy_swapchain(mem::take(&mut self.swapchain.raw), None);
        for frame in self.frames.drain(..) {
            raw_device.destroy_image_view(frame.view, None);
            raw_device.destroy_semaphore(frame.acquire_semaphore, None);
            raw_device.destroy_semaphore(frame.present_semaphore, None);
        }
    }

    pub fn acquire_frame(&mut self) -> super::Frame {
        match self.try_acquire() {
            Ok(frame) => frame,
            // wpe-gallery patch: upstream 0.7.1 returned an unpresentable frame
            // forever, so every later draw was silently dropped until the
            // application happened to resize. NVIDIA's Wayland WSI reports
            // OUT_OF_DATE after the initial configure on Smithay compositors
            // (niri). The swapchain is no longer usable: rebuild it with the
            // last configuration and acquire again.
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                // One rebuild per healthy swapchain: if the rebuilt one is out
                // of date before it ever acquired, wait for the application's
                // own reconfigure instead of rebuilding on every draw. Also
                // leave fixed-extent surfaces (X11) whose size changed to the
                // application's resize, as the old size would be invalid.
                match self.config {
                    Some(config) if !self.recovery_pending && self.extent_still_valid(config) => {
                        log::warn!(
                            "Acquire failed because the surface is out of date; recreating swapchain"
                        );
                        unsafe { self.configure(config) };
                        self.recovery_pending = true;
                        match self.try_acquire() {
                            Ok(frame) => return frame,
                            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                                log::warn!("Recreated swapchain is still out of date; frame dropped")
                            }
                            Err(other) => panic!("Aquire image error {}", other),
                        }
                    }
                    _ => log::debug!("Acquire failed because the surface is out of date"),
                }
                super::Frame {
                    internal: self.frames[0],
                    swapchain: self.swapchain,
                    image_index: None,
                }
            }
            Err(other) => panic!("Aquire image error {}", other),
        }
    }

    fn extent_still_valid(&self, config: crate::SurfaceConfig) -> bool {
        let capabilities = unsafe {
            self.factory
                .surface_instance
                .get_physical_device_surface_capabilities(self.factory.physical_device, self.raw)
        };
        match capabilities {
            Ok(c) => {
                c.current_extent.width == u32::MAX
                    || (c.current_extent.width == config.size.width
                        && c.current_extent.height == config.size.height)
            }
            Err(_) => false,
        }
    }

    fn try_acquire(&mut self) -> Result<super::Frame, vk::Result> {
        let acquire_semaphore = self.next_semaphore;
        // A failed acquire leaves the semaphore unsignaled, so it can be reused.
        let (index, _suboptimal) = unsafe {
            self.device.acquire_next_image(
                self.swapchain.raw,
                !0,
                acquire_semaphore,
                vk::Fence::null(),
            )
        }?;
        self.next_semaphore = mem::replace(
            &mut self.frames[index as usize].acquire_semaphore,
            acquire_semaphore,
        );
        self.recovery_pending = false;
        Ok(super::Frame {
            internal: self.frames[index as usize],
            swapchain: self.swapchain,
            image_index: Some(index),
        })
    }
}

impl super::Context {
    pub fn create_surface<
        I: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle,
    >(
        &self,
        window: &I,
    ) -> Result<super::Surface, crate::NotSupportedError> {
        let khr_swapchain = self
            .device
            .swapchain
            .clone()
            .ok_or(crate::NotSupportedError::NoSupportedDeviceFound)?;

        let raw = unsafe {
            ash_window::create_surface(
                &self.entry,
                &self.instance.core,
                window.display_handle().unwrap().as_raw(),
                window.window_handle().unwrap().as_raw(),
                None,
            )
            .map_err(super::PlatformError::Init)?
        };

        let khr_surface = self
            .instance
            .surface
            .as_ref()
            .ok_or(crate::NotSupportedError::PlatformNotSupported)?;
        if unsafe {
            khr_surface.get_physical_device_surface_support(
                self.physical_device,
                self.queue_family_index,
                raw,
            ) != Ok(true)
        } {
            log::warn!("Rejected for not presenting to the window surface");
            return Err(crate::NotSupportedError::PlatformNotSupported);
        }

        let mut surface_info = vk::PhysicalDeviceSurfaceInfo2KHR {
            surface: raw,
            ..Default::default()
        };
        let mut fullscreen_exclusive_win32 = vk::SurfaceFullScreenExclusiveWin32InfoEXT::default();
        surface_info = surface_info.push_next(&mut fullscreen_exclusive_win32);
        let mut fullscreen_exclusive_ext = vk::SurfaceCapabilitiesFullScreenExclusiveEXT::default();
        let mut capabilities2_khr =
            vk::SurfaceCapabilities2KHR::default().push_next(&mut fullscreen_exclusive_ext);
        let _ = unsafe {
            self.instance
                .get_surface_capabilities2
                .as_ref()
                .unwrap()
                .get_physical_device_surface_capabilities2(
                    self.physical_device,
                    &surface_info,
                    &mut capabilities2_khr,
                )
        };
        log::debug!("{:?}", capabilities2_khr.surface_capabilities);

        let semaphore_create_info = vk::SemaphoreCreateInfo::default();
        let next_semaphore = unsafe {
            self.device
                .core
                .create_semaphore(&semaphore_create_info, None)
                .unwrap()
        };

        Ok(super::Surface {
            factory: super::SwapchainFactory {
                surface_instance: khr_surface.clone(),
                core: self.device.core.clone(),
                physical_device: self.physical_device,
                queue_family_index: self.queue_family_index,
                full_screen_exclusive_device: self.device.full_screen_exclusive.is_some(),
            },
            config: None,
            recovery_pending: false,
            device: khr_swapchain,
            raw,
            frames: Vec::new(),
            next_semaphore,
            swapchain: super::Swapchain {
                raw: vk::SwapchainKHR::null(),
                format: crate::TextureFormat::Rgba8Unorm,
                alpha: crate::AlphaMode::Ignored,
                target_size: [0; 2],
            },
            full_screen_exclusive: fullscreen_exclusive_ext.full_screen_exclusive_supported != 0,
        })
    }

    pub fn destroy_surface(&self, surface: &mut super::Surface) {
        unsafe {
            surface.deinit_swapchain(&self.device.core);
            self.device
                .core
                .destroy_semaphore(surface.next_semaphore, None)
        };
        if let Some(ref surface_instance) = self.instance.surface {
            unsafe { surface_instance.destroy_surface(surface.raw, None) };
        }
    }

    pub fn reconfigure_surface(&self, surface: &mut super::Surface, config: crate::SurfaceConfig) {
        unsafe { surface.configure(config) };
        surface.recovery_pending = false;
    }
}

impl super::Surface {
    /// Creates (or replaces) the swapchain. Shared by explicit reconfiguration
    /// and out-of-date recovery in `acquire_frame`. As upstream does on resize,
    /// the old swapchain's present semaphores are destroyed after
    /// device_wait_idle; without VK_EXT_swapchain_maintenance1 there is no
    /// presentation-engine fence to wait on.
    unsafe fn configure(&mut self, config: crate::SurfaceConfig) {
        let factory = self.factory.clone();
        let khr_surface = &factory.surface_instance;
        let surface = self;

        let capabilities = unsafe {
            khr_surface
                .get_physical_device_surface_capabilities(factory.physical_device, surface.raw)
                .unwrap()
        };
        if config.size.width < capabilities.min_image_extent.width
            || config.size.width > capabilities.max_image_extent.width
            || config.size.height < capabilities.min_image_extent.height
            || config.size.height > capabilities.max_image_extent.height
        {
            log::warn!(
                "Requested size {}x{} is outside of surface capabilities",
                config.size.width,
                config.size.height
            );
        }

        let (alpha, composite_alpha) = if config.transparent {
            if capabilities
                .supported_composite_alpha
                .contains(vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED)
            {
                (
                    crate::AlphaMode::PostMultiplied,
                    vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
                )
            } else if capabilities
                .supported_composite_alpha
                .contains(vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED)
            {
                (
                    crate::AlphaMode::PreMultiplied,
                    vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
                )
            } else {
                log::error!(
                    "No composite alpha flag for transparency: {:?}",
                    capabilities.supported_composite_alpha
                );
                (
                    crate::AlphaMode::Ignored,
                    vk::CompositeAlphaFlagsKHR::OPAQUE,
                )
            }
        } else {
            (
                crate::AlphaMode::Ignored,
                vk::CompositeAlphaFlagsKHR::OPAQUE,
            )
        };

        let (requested_frame_count, mode_preferences) = match config.display_sync {
            crate::DisplaySync::Block => (3, [vk::PresentModeKHR::FIFO].as_slice()),
            crate::DisplaySync::Recent => (
                3,
                [
                    vk::PresentModeKHR::MAILBOX,
                    vk::PresentModeKHR::FIFO_RELAXED,
                    vk::PresentModeKHR::IMMEDIATE,
                ]
                .as_slice(),
            ),
            crate::DisplaySync::Tear => (2, [vk::PresentModeKHR::IMMEDIATE].as_slice()),
        };
        let effective_frame_count = requested_frame_count.max(capabilities.min_image_count);

        let present_modes = unsafe {
            khr_surface
                .get_physical_device_surface_present_modes(factory.physical_device, surface.raw)
                .unwrap()
        };
        let present_mode = *mode_preferences
            .iter()
            .find(|mode| present_modes.contains(mode))
            .unwrap();
        log::info!("Using surface present mode {:?}", present_mode);

        let queue_families = [factory.queue_family_index];

        let mut supported_formats = Vec::new();
        let (format, surface_format) = if surface.swapchain.target_size[0] > 0 {
            let format = surface.swapchain.format;
            log::info!("Retaining current format: {:?}", format);
            let vk_color_space = match (format, config.color_space) {
                (crate::TextureFormat::Bgra8Unorm, crate::ColorSpace::Srgb) => {
                    vk::ColorSpaceKHR::SRGB_NONLINEAR
                }
                (crate::TextureFormat::Bgra8Unorm, crate::ColorSpace::Linear) => {
                    vk::ColorSpaceKHR::EXTENDED_SRGB_LINEAR_EXT
                }
                (crate::TextureFormat::Bgra8UnormSrgb, crate::ColorSpace::Linear) => {
                    vk::ColorSpaceKHR::default()
                }
                _ => panic!(
                    "Unexpected format {:?} under color space {:?}",
                    format, config.color_space
                ),
            };
            (
                format,
                vk::SurfaceFormatKHR {
                    format: super::map_texture_format(format),
                    color_space: vk_color_space,
                },
            )
        } else {
            supported_formats = unsafe {
                khr_surface
                    .get_physical_device_surface_formats(factory.physical_device, surface.raw)
                    .unwrap()
            };
            match config.color_space {
                crate::ColorSpace::Linear => {
                    let surface_format = vk::SurfaceFormatKHR {
                        format: vk::Format::B8G8R8A8_UNORM,
                        color_space: vk::ColorSpaceKHR::EXTENDED_SRGB_LINEAR_EXT,
                    };
                    if supported_formats.contains(&surface_format) {
                        log::info!("Using linear SRGB color space");
                        (crate::TextureFormat::Bgra8Unorm, surface_format)
                    } else {
                        (
                            crate::TextureFormat::Bgra8UnormSrgb,
                            vk::SurfaceFormatKHR {
                                format: vk::Format::B8G8R8A8_SRGB,
                                color_space: vk::ColorSpaceKHR::default(),
                            },
                        )
                    }
                }
                crate::ColorSpace::Srgb => (
                    crate::TextureFormat::Bgra8Unorm,
                    vk::SurfaceFormatKHR {
                        format: vk::Format::B8G8R8A8_UNORM,
                        color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
                    },
                ),
            }
        };
        if !supported_formats.is_empty() && !supported_formats.contains(&surface_format) {
            log::error!("Surface formats are incompatible: {:?}", supported_formats);
        }

        let vk_usage = super::resource::map_texture_usage(config.usage, crate::TexelAspects::COLOR);
        if !capabilities.supported_usage_flags.contains(vk_usage) {
            log::error!(
                "Surface usages are incompatible: {:?}",
                capabilities.supported_usage_flags
            );
        }

        let mut full_screen_exclusive_info = vk::SurfaceFullScreenExclusiveInfoEXT {
            full_screen_exclusive: if config.allow_exclusive_full_screen {
                vk::FullScreenExclusiveEXT::ALLOWED
            } else {
                vk::FullScreenExclusiveEXT::DISALLOWED
            },
            ..Default::default()
        };

        let mut create_info = vk::SwapchainCreateInfoKHR {
            surface: surface.raw,
            min_image_count: effective_frame_count,
            image_format: surface_format.format,
            image_color_space: surface_format.color_space,
            image_extent: vk::Extent2D {
                width: config.size.width,
                height: config.size.height,
            },
            image_array_layers: 1,
            image_usage: vk_usage,
            pre_transform: vk::SurfaceTransformFlagsKHR::IDENTITY,
            composite_alpha,
            present_mode,
            old_swapchain: surface.swapchain.raw,
            ..Default::default()
        }
        .queue_family_indices(&queue_families);

        if surface.full_screen_exclusive {
            assert!(factory.full_screen_exclusive_device);
            create_info = create_info.push_next(&mut full_screen_exclusive_info);
            log::info!(
                "Configuring exclusive full screen: {}",
                config.allow_exclusive_full_screen
            );
        }
        let raw_swapchain = unsafe { surface.device.create_swapchain(&create_info, None).unwrap() };

        unsafe {
            surface.deinit_swapchain(&factory.core);
        }

        let images = unsafe { surface.device.get_swapchain_images(raw_swapchain).unwrap() };
        let target_size = [config.size.width as u16, config.size.height as u16];
        let subresource_range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        for image in images {
            let view_create_info = vk::ImageViewCreateInfo {
                image,
                view_type: vk::ImageViewType::TYPE_2D,
                format: surface_format.format,
                subresource_range,
                ..Default::default()
            };
            let view = unsafe {
                factory.core
                    .create_image_view(&view_create_info, None)
                    .unwrap()
            };
            let semaphore_create_info = vk::SemaphoreCreateInfo::default();
            let acquire_semaphore = unsafe {
                factory.core
                    .create_semaphore(&semaphore_create_info, None)
                    .unwrap()
            };
            let present_semaphore = unsafe {
                factory.core
                    .create_semaphore(&semaphore_create_info, None)
                    .unwrap()
            };
            surface.frames.push(super::InternalFrame {
                acquire_semaphore,
                present_semaphore,
                image,
                view,
            });
        }
        surface.swapchain = super::Swapchain {
            raw: raw_swapchain,
            format,
            alpha,
            target_size,
        };
        surface.config = Some(config);
    }
}
