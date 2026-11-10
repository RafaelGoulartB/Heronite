//! The graphics adapters DXGI lists, in the order it gives them for a GPU
//! preference (`IDXGIFactory6::EnumAdapterByGpuPreference`). The first one of
//! each order is the card Windows gives a game set to "Power saving" or to
//! "High performance" in Settings → Display → Graphics (the per-program
//! `GpuPreference` value).

use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory6, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_GPU_PREFERENCE,
    DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE, DXGI_GPU_PREFERENCE_MINIMUM_POWER,
};

/// One hardware adapter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Adapter {
    /// The driver's description without the (R) / (TM) marks:
    /// "NVIDIA GeForce RTX 4070 Laptop GPU".
    pub name: String,
    /// PCI ids.
    pub vendor_id: u32,
    pub device_id: u32,
    /// Video memory of its own, in bytes (0 for most integrated cards).
    pub dedicated_memory: u64,
}

/// The adapters in the two orders of a GPU preference.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Adapters {
    /// Integrated first, where there is one.
    pub power_saving: Vec<Adapter>,
    /// The fastest first.
    pub high_performance: Vec<Adapter>,
}

impl Adapters {
    /// Asks DXGI. Empty when it cannot be asked (Windows before 10 1803 has
    /// no `IDXGIFactory6`).
    pub fn detect() -> Self {
        // SAFETY: no arguments; the factory is released when dropped.
        let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory6>() }) else { return Self::default() };
        Self {
            power_saving: ordered(&factory, DXGI_GPU_PREFERENCE_MINIMUM_POWER),
            high_performance: ordered(&factory, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE),
        }
    }
}

/// The hardware adapters in `preference`'s order: WARP and the virtual
/// displays some apps add are left out.
fn ordered(factory: &IDXGIFactory6, preference: DXGI_GPU_PREFERENCE) -> Vec<Adapter> {
    let mut out = Vec::new();
    for index in 0.. {
        // SAFETY: a valid factory; DXGI_ERROR_NOT_FOUND ends the list.
        let Ok(adapter) = (unsafe { factory.EnumAdapterByGpuPreference::<IDXGIAdapter1>(index, preference) }) else { break };
        // SAFETY: a valid adapter.
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else { continue };
        if desc.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
            continue;
        }
        let end = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
        let name = crate::system::tidy(&String::from_utf16_lossy(&desc.Description[..end]));
        if name.is_empty() || crate::system::is_software_adapter(&name) {
            continue;
        }
        let a = Adapter { name, vendor_id: desc.VendorId, device_id: desc.DeviceId, dedicated_memory: desc.DedicatedVideoMemory as u64 };
        // An adapter with two outputs on separate nodes can be listed twice.
        if !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the real adapters (nothing is changed): both orders hold the
    /// same cards, and none is a software one.
    #[test]
    fn both_orders_list_the_same_hardware_adapters() {
        let a = Adapters::detect();
        let mut p: Vec<_> = a.power_saving.iter().map(|x| x.name.clone()).collect();
        let mut h: Vec<_> = a.high_performance.iter().map(|x| x.name.clone()).collect();
        p.sort();
        h.sort();
        assert_eq!(p, h);
        assert!(a.power_saving.iter().all(|x| !x.name.contains("Basic Render")), "{a:?}");
    }
}
