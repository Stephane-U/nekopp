// Neko++ -- a GPU-rendered text editor.
// Copyright (C) 2026 Stéphane Urdy
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU General Public License as published by the Free
// Software Foundation, version 3 of the License.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or
// FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for
// more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.

//! The GPU context: instance, adapter, device, queue, and surface.
//!
//! Kept separate from the window so the renderer can be swapped or tested without
//! dragging the event loop along.
//!
//! # Why there is a ladder
//!
//! `request_adapter` with `HighPerformance` is the obvious call and it is the
//! wrong one on its own: it asks for a discrete GPU, and a machine with only an
//! integrated one, or only Mesa's software rasterizer, answers "no adapter" and
//! the editor refuses to start. The failure is not a missing GPU, it is a
//! question that was too specific.
//!
//! So the adapter is asked for in steps, from most to least desirable, and the
//! first answer wins. The last step asks for a *fallback* adapter, which is how
//! wgpu spells "software rasterizer": lavapipe on Vulkan, llvmpipe on GL. That
//! is slow, but a slow editor beats no editor, and it is the only path that
//! works on a machine with no GPU driver at all.
//!
//! The ladder is a plain function returning a plain list, so the order can be
//! tested without a GPU. Only the last step needs hardware to verify, and that
//! is what `--software` is for.

use std::fmt;
use std::sync::Arc;

use winit::window::Window;

/// One rung of the adapter ladder.
///
/// Deliberately not `wgpu::PowerPreference`: the software rung is not a power
/// preference, it is a different question (`force_fallback_adapter`), and
/// folding the two into one enum keeps the ladder readable as a list of
/// intentions rather than a list of option structs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rung {
    /// The discrete GPU, if the machine has one.
    Fast,
    /// The integrated GPU, or whatever is left after the discrete one.
    Slow,
    /// Mesa's software rasterizer. Correct, and slow.
    Software,
}

/// The two fields of an adapter request a rung decides.
///
/// A struct rather than `wgpu::RequestAdapterOptions` because that type is
/// generic over the surface, and the surface is the one part of the request the
/// ladder has no opinion about. Keeping the rung's half separate is what lets
/// the order be tested without a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// Which kind of GPU to prefer.
    pub power: wgpu::PowerPreference,
    /// Whether to accept only a software rasterizer.
    pub fallback: bool,
}

impl Rung {
    /// The adapter request this rung makes.
    pub fn request(self) -> Request {
        match self {
            Rung::Fast => Request {
                power: wgpu::PowerPreference::HighPerformance,
                fallback: false,
            },
            Rung::Slow => Request {
                power: wgpu::PowerPreference::LowPower,
                fallback: false,
            },
            Rung::Software => Request {
                power: wgpu::PowerPreference::LowPower,
                fallback: true,
            },
        }
    }

    /// How the rung is named in a log line or an error.
    pub fn label(self) -> &'static str {
        match self {
            Rung::Fast => "high-performance",
            Rung::Slow => "low-power",
            Rung::Software => "software",
        }
    }
}

/// The order the adapter is asked for in.
///
/// `force_software` collapses the ladder to its last rung. That is what
/// `--software` does, and it exists because the software path is the one that
/// is hardest to test by accident: on a machine with a working GPU the first
/// rung always wins, so without a way to skip to the end the fallback would only
/// ever run on hardware nobody has.
pub fn ladder(force_software: bool) -> Vec<Rung> {
    if force_software {
        vec![Rung::Software]
    } else {
        vec![Rung::Fast, Rung::Slow, Rung::Software]
    }
}

/// Why a GPU context could not be built.
///
/// A type rather than a `String` because the caller has to decide between
/// "report and exit" and "report and keep going", and because the message is
/// long enough that building it at the point of failure would bury the logic.
#[derive(Debug)]
pub enum GpuError {
    /// The window could not be turned into a surface.
    Surface(String),
    /// An adapter was found but no device could be opened from it.
    Device(String),
    /// No rung of the ladder produced an adapter.
    NoAdapter {
        /// The rungs that were tried, in order.
        tried: Vec<Rung>,
        /// Every adapter the instance could see, for the bug report.
        seen: Vec<String>,
    },
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuError::Surface(err) => write!(f, "cannot draw to this window: {err}"),
            GpuError::Device(err) => write!(f, "cannot open the GPU device: {err}"),
            GpuError::NoAdapter { tried, seen } => {
                let names: Vec<&str> = tried.iter().map(|rung| rung.label()).collect();
                writeln!(f, "no usable GPU adapter (tried {})", names.join(", "))?;
                // Two failures look the same from here and need different
                // advice. No adapter at all means no driver, and the fix is an
                // install. An adapter that cannot present to this window means
                // the driver is there but the surface is not, and installing
                // another one will not help.
                if seen.is_empty() {
                    writeln!(f, "  no adapters were visible at all")?;
                    // The software rasterizer is a package, not something that
                    // ships with the driver, so the fix is an install and the
                    // message has to say which one.
                    write!(
                        f,
                        "  install a GPU driver, or Mesa's software rasterizer:\n\
                         \x20   Debian/Ubuntu  apt install mesa-vulkan-drivers\n\
                         \x20   Fedora         dnf install mesa-vulkan-drivers\n\
                         \x20   Arch           pacman -S vulkan-swrast\n\
                         \x20 then run again, or pass --software to force that path"
                    )
                } else {
                    writeln!(f, "  adapters visible: {}", seen.join(", "))?;
                    write!(
                        f,
                        "  none of them can present to this window; a driver is \
                         installed, so the surface is the problem.\n\
                         \x20 try --software, or run under a plain X11 session \
                         rather than a nested one"
                    )
                }
            }
        }
    }
}

/// Everything needed to submit work to the GPU and present to a window.
pub struct Gpu {
    /// The swapchain surface.
    pub surface: wgpu::Surface<'static>,
    /// The logical device.
    pub device: wgpu::Device,
    /// The submission queue.
    pub queue: wgpu::Queue,
    /// The current surface configuration.
    pub config: wgpu::SurfaceConfiguration,
    /// The adapter the device came from.
    pub adapter: wgpu::Adapter,
    /// The instance the surface was created from.
    ///
    /// Kept only so a lost surface can be recreated. wgpu's own documentation
    /// for `CurrentSurfaceTexture::Lost` says to call `Instance::create_surface`
    /// and then configure -- reconfiguring the lost surface is not enough,
    /// because the thing that was lost is the surface itself. Without the
    /// instance there is no way to make a new one, and the window would stay
    /// blank for the rest of the session.
    instance: wgpu::Instance,
    /// The window the surface draws into, for the same reason.
    window: Arc<Window>,
    /// Which rung of the ladder answered.
    rung: Rung,
}

impl Gpu {
    /// Opens a surface on `window` and picks an adapter for it.
    ///
    /// `force_software` skips straight to the software rung. Everything else
    /// walks the ladder in [`ladder`] order and stops at the first adapter that
    /// also accepts a device: an adapter that cannot open a device is no more
    /// use than no adapter, so it does not end the search.
    pub fn new(window: Arc<Window>, force_software: bool) -> Result<Self, GpuError> {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        // Cloned rather than moved: the surface keeps its own handle, and `Gpu`
        // keeps this one so `recover` can build a replacement surface later.
        let surface = instance
            .create_surface(window.clone())
            .map_err(|err| GpuError::Surface(err.to_string()))?;

        let tried = ladder(force_software);
        let mut last_device_error: Option<String> = None;
        let mut found: Option<(wgpu::Adapter, wgpu::Device, wgpu::Queue, Rung)> = None;
        for rung in &tried {
            let request = rung.request();
            let options = wgpu::RequestAdapterOptions {
                power_preference: request.power,
                force_fallback_adapter: request.fallback,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            };
            let Ok(adapter) = pollster::block_on(instance.request_adapter(&options)) else {
                continue;
            };
            // A device request can fail on an adapter that enumerated fine --
            // a driver that advertises a feature it cannot deliver, or a
            // container with no render node. Keep the message and try the next
            // rung rather than giving up on the first refusal.
            match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("npp device"),
                ..Default::default()
            })) {
                Ok((device, queue)) => {
                    found = Some((adapter, device, queue, *rung));
                    break;
                }
                Err(err) => last_device_error = Some(err.to_string()),
            }
        }

        let Some((adapter, device, queue, rung)) = found else {
            // Only enumerate when the ladder failed: it is a second round trip
            // to the driver and the answer is only wanted for the error.
            let seen = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
                .iter()
                .map(|adapter| {
                    let info = adapter.get_info();
                    format!("{} ({:?})", info.name, info.backend)
                })
                .collect();
            return Err(match last_device_error {
                Some(err) => GpuError::Device(err),
                None => GpuError::NoAdapter { tried, seen },
            });
        };

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            // Fifo is the only present mode guaranteed to exist, and it is the one
            // that paces us to the display's configured refresh rate.
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            // One frame in flight: the CPU never queues ahead of the GPU. We are
            // far from GPU-bound, so there is nothing to overlap, and this
            // removes up to a frame of input-to-photon delay.
            desired_maximum_frame_latency: 1,
        };
        surface.configure(&device, &config);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            adapter,
            instance,
            window,
            rung,
        })
    }

    /// The surface's texture format.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// Which rung of the ladder answered.
    pub fn rung(&self) -> Rung {
        self.rung
    }

    /// Whether this context is running on a software rasterizer.
    ///
    /// Read from the adapter rather than from the rung: a driver can hand back
    /// a software adapter for a hardware request, and the status bar should say
    /// what is actually drawing.
    pub fn is_software(&self) -> bool {
        self.adapter.get_info().device_type == wgpu::DeviceType::Cpu
    }

    /// Reconfigures the surface for a new window size.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Rebuilds the surface after the driver reported it lost.
    ///
    /// Returns whether the window can be drawn into again. A failure here is not
    /// fatal to the process -- the document is still in memory and the user can
    /// still save it -- so the caller logs and keeps going rather than exiting.
    ///
    /// The order matters and is the one wgpu's documentation gives: create a new
    /// surface, then configure it. Configuring the old one would be a no-op on a
    /// surface the driver has already discarded.
    pub fn recover(&mut self) -> bool {
        let surface = match self.instance.create_surface(self.window.clone()) {
            Ok(surface) => surface,
            Err(err) => {
                eprintln!("npp: cannot recreate the surface: {err}");
                return false;
            }
        };
        // The new surface may not support the format the old one did -- a window
        // dragged to a different output can land on a different driver path --
        // so the format is renegotiated rather than assumed.
        let caps = surface.get_capabilities(&self.adapter);
        if caps.formats.is_empty() {
            eprintln!("npp: the recreated surface has no formats");
            return false;
        }
        self.config.format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        self.config.alpha_mode = caps.alpha_modes[0];
        surface.configure(&self.device, &self.config);
        self.surface = surface;
        true
    }

    /// A one-line description of the adapter, for the status bar and logs.
    pub fn describe(&self) -> String {
        let info = self.adapter.get_info();
        format!("{} ({:?})", info.name, info.backend)
    }

    /// The present modes this surface supports, for diagnostics.
    pub fn present_modes(&self) -> Vec<wgpu::PresentMode> {
        self.surface
            .get_capabilities(&self.adapter)
            .present_modes
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ladder_prefers_hardware_and_ends_with_software() {
        assert_eq!(ladder(false), vec![Rung::Fast, Rung::Slow, Rung::Software]);
    }

    #[test]
    fn forcing_software_collapses_the_ladder() {
        assert_eq!(ladder(true), vec![Rung::Software]);
    }

    #[test]
    fn only_the_last_rung_asks_for_a_fallback_adapter() {
        assert!(!Rung::Fast.request().fallback);
        assert!(!Rung::Slow.request().fallback);
        assert!(Rung::Software.request().fallback);
    }

    #[test]
    fn the_first_rung_asks_for_the_discrete_gpu() {
        assert_eq!(
            Rung::Fast.request().power,
            wgpu::PowerPreference::HighPerformance
        );
        // The software rung must not ask for high performance: a fallback
        // adapter is by definition not the fast one, and asking for both is a
        // contradiction some backends resolve by returning nothing.
        assert_eq!(
            Rung::Software.request().power,
            wgpu::PowerPreference::LowPower
        );
    }

    #[test]
    fn every_rung_has_a_distinct_label() {
        let labels: Vec<&str> = ladder(false).iter().map(|rung| rung.label()).collect();
        assert_eq!(labels, vec!["high-performance", "low-power", "software"]);
    }

    #[test]
    fn the_no_adapter_message_names_the_rungs_and_the_fix() {
        let err = GpuError::NoAdapter {
            tried: ladder(false),
            seen: Vec::new(),
        };
        let text = err.to_string();
        assert!(text.contains("high-performance, low-power, software"), "{text}");
        assert!(text.contains("no adapters were visible"), "{text}");
        assert!(text.contains("mesa-vulkan-drivers"), "{text}");
        assert!(text.contains("--software"), "{text}");
    }

    #[test]
    fn the_no_adapter_message_lists_the_adapters_it_could_see() {
        let err = GpuError::NoAdapter {
            tried: ladder(true),
            seen: vec!["llvmpipe (Vulkan)".to_string()],
        };
        let text = err.to_string();
        assert!(text.contains("adapters visible: llvmpipe (Vulkan)"), "{text}");
        assert!(!text.contains("no adapters were visible"), "{text}");
    }

    #[test]
    fn a_visible_adapter_gets_surface_advice_not_install_advice() {
        // The two failures need different fixes: a missing driver is an
        // install, an adapter that cannot present is not. Telling someone to
        // install mesa when mesa is already there sends them down a dead end.
        let err = GpuError::NoAdapter {
            tried: ladder(false),
            seen: vec!["NVIDIA GeForce GTX 1080 Ti (Gl)".to_string()],
        };
        let text = err.to_string();
        assert!(text.contains("none of them can present"), "{text}");
        assert!(text.contains("--software"), "{text}");
        assert!(!text.contains("apt install"), "{text}");
    }

    #[test]
    fn a_device_failure_is_reported_as_a_device_failure() {
        let text = GpuError::Device("out of memory".to_string()).to_string();
        assert!(text.contains("out of memory"), "{text}");
        assert!(!text.contains("mesa-vulkan-drivers"), "{text}");
    }
}
