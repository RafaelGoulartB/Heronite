//! Blocking Windows Core Audio controls. Every call joins COM on the calling
//! worker for its duration and owns its interfaces; only plain snapshot data
//! leaves it.
//!
//! Changing the default device has no documented API: [`set_default_device`]
//! uses IPolicyConfig ([`policy`]), what Windows' own sound settings call and what
//! EarTrumpet, SoundSwitch and AudioDeviceCmdlets use, unchanged since Vista.

use std::io;
use std::ptr;

use windows::core::{Error, Result, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eCapture, eCommunications, eConsole, eMultimedia, eRender, EDataFlow, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, STGM_READ};

use crate::com::Com;
use crate::process::ProcessEntry;

mod sessions;
pub use sessions::{set_process_sessions_muted, set_process_sessions_volume, set_sessions_muted, set_sessions_volume, Session, SessionId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Percent, limited to 0..=100 by Windows.
    pub volume: u16,
    pub muted: bool,
    pub outputs: Vec<Device>,
    pub default_output: Option<String>,
    pub mic_muted: Option<bool>,
    pub inputs: Vec<Device>,
    pub default_input: Option<String>,
    pub sessions: Vec<Session>,
}

/// Output sessions, playing or silent for now (as Windows' Volume Mixer
/// lists them), and the default multimedia endpoints. `processes` (a
/// [`crate::process::snapshot`]) names the sessions' executables.
pub fn read(processes: &[ProcessEntry]) -> io::Result<Snapshot> {
    with_audio(|enumerator| {
        let output = default_device(enumerator, eRender)?;
        let (volume, muted) = match &output {
            Some(device) => {
                let volume = endpoint_volume(device)?;
                // SAFETY: all interfaces remain on this initialized worker.
                unsafe { (percent(volume.GetMasterVolumeLevelScalar()?), volume.GetMute()?.as_bool()) }
            }
            None => (0, false),
        };
        let microphone = microphone(enumerator);
        let outputs = devices(enumerator, eRender)?;
        let sessions = sessions::read(&outputs, processes);
        Ok(Snapshot {
            volume,
            muted,
            outputs: outputs.into_iter().map(|(device, _)| device).collect(),
            default_output: output.as_ref().map(device_id).transpose()?,
            mic_muted: microphone.muted,
            inputs: microphone.inputs,
            default_input: microphone.default,
            sessions,
        })
    })
}

/// The capture side of [`read`].
struct Microphone {
    muted: Option<bool>,
    inputs: Vec<Device>,
    default: Option<String>,
}

/// The capture side, read apart from the outputs: what fails here counts as
/// no microphone (logged), so the output controls keep working.
fn microphone(enumerator: &IMMDeviceEnumerator) -> Microphone {
    let input = default_device(enumerator, eCapture).unwrap_or_else(|error| {
        log::warn!("Core Audio: reading the default microphone: {error}");
        None
    });
    let muted = input.as_ref().and_then(|device| match endpoint_muted(device) {
        Ok(muted) => Some(muted),
        Err(error) => {
            log::warn!("Core Audio: reading microphone mute: {error}");
            None
        }
    });
    let default = input.as_ref().and_then(|device| match device_id(device) {
        Ok(id) => Some(id),
        Err(error) => {
            log::warn!("Core Audio: reading the default microphone's ID: {error}");
            None
        }
    });
    let inputs = match devices(enumerator, eCapture) {
        Ok(inputs) => inputs.into_iter().map(|(device, _)| device).collect(),
        Err(error) => {
            log::warn!("Core Audio: listing microphones: {error}");
            Vec::new()
        }
    };
    Microphone { muted, inputs, default }
}

/// Sets the default output's volume and unmutes it. Values above 100 clamp.
pub fn set_volume(percent: u16) -> io::Result<()> {
    with_audio(|enumerator| {
        let volume = default_volume(enumerator, eRender)?;
        // SAFETY: `volume` belongs to this initialized worker; no event context.
        unsafe {
            volume.SetMasterVolumeLevelScalar(scalar(percent), ptr::null())?;
            volume.SetMute(false, ptr::null())
        }
    })
}

/// Reads and changes the same endpoint, even if the default changes meanwhile.
pub fn step_volume(delta: i32) -> io::Result<u16> {
    with_audio(|enumerator| {
        let volume = default_volume(enumerator, eRender)?;
        // SAFETY: `volume` belongs to this initialized worker.
        unsafe {
            let next = stepped(percent(volume.GetMasterVolumeLevelScalar()?), delta);
            volume.SetMasterVolumeLevelScalar(scalar(next), ptr::null())?;
            volume.SetMute(false, ptr::null())?;
            Ok(next)
        }
    })
}

pub fn set_muted(muted: bool) -> io::Result<()> {
    set_endpoint_muted(eRender, muted)
}

pub fn set_mic_muted(muted: bool) -> io::Result<()> {
    set_endpoint_muted(eCapture, muted)
}

/// Makes the endpoint `id` (a [`Device::id`] of [`Snapshot::outputs`] or
/// [`Snapshot::inputs`]) the default for every role, as the sound settings
/// do: games, media and calls follow it. Streams playing on the old default
/// move with it, unless their app picked a device itself.
pub fn set_default_device(id: &str) -> io::Result<()> {
    let id: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
    with_com(|| {
        // SAFETY: COM is initialized for this call; `id` is NUL-terminated and
        // outlives every call that borrows it.
        unsafe {
            let config: policy::IPolicyConfig = CoCreateInstance(&policy::CLIENT, None, CLSCTX_ALL)?;
            for role in [eConsole, eMultimedia, eCommunications] {
                config.SetDefaultEndpoint(PCWSTR(id.as_ptr()), role).ok()?;
            }
        }
        Ok(())
    })
}

/// The undocumented policy interface of the audio service, in its Windows 7+
/// layout. Only `SetDefaultEndpoint` is called; the other methods keep its
/// place in the vtable.
#[allow(dead_code, non_snake_case)]
mod policy {
    use std::ffi::c_void;

    use windows::core::{GUID, HRESULT, PCWSTR};
    use windows::Win32::Media::Audio::ERole;
    use windows_core::{interface, IUnknown, IUnknown_Vtbl};

    /// CPolicyConfigClient, the class behind [`IPolicyConfig`].
    pub(super) const CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

    #[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    pub(super) unsafe trait IPolicyConfig: IUnknown {
        fn GetMixFormat(&self, device: PCWSTR, format: *mut *mut c_void) -> HRESULT;
        fn GetDeviceFormat(&self, device: PCWSTR, default: i32, format: *mut *mut c_void) -> HRESULT;
        fn ResetDeviceFormat(&self, device: PCWSTR) -> HRESULT;
        fn SetDeviceFormat(&self, device: PCWSTR, endpoint: *mut c_void, mix: *mut c_void) -> HRESULT;
        fn GetProcessingPeriod(&self, device: PCWSTR, default: i32, period: *mut i64, minimum: *mut i64) -> HRESULT;
        fn SetProcessingPeriod(&self, device: PCWSTR, period: *mut i64) -> HRESULT;
        fn GetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn SetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn GetPropertyValue(&self, device: PCWSTR, key: *const c_void, value: *mut c_void) -> HRESULT;
        fn SetPropertyValue(&self, device: PCWSTR, key: *const c_void, value: *mut c_void) -> HRESULT;
        pub(super) fn SetDefaultEndpoint(&self, device: PCWSTR, role: ERole) -> HRESULT;
        fn SetEndpointVisibility(&self, device: PCWSTR, visible: i32) -> HRESULT;
    }
}

fn set_endpoint_muted(flow: EDataFlow, muted: bool) -> io::Result<()> {
    with_audio(|enumerator| {
        let volume = default_volume(enumerator, flow)?;
        // SAFETY: the interface is live on this COM worker.
        unsafe { volume.SetMute(muted, ptr::null()) }
    })
}

fn default_volume(enumerator: &IMMDeviceEnumerator, flow: EDataFlow) -> Result<IAudioEndpointVolume> {
    let device = default_device(enumerator, flow)?.ok_or_else(|| not_found("No default audio endpoint"))?;
    endpoint_volume(&device)
}

fn endpoint_volume(device: &IMMDevice) -> Result<IAudioEndpointVolume> {
    // SAFETY: the device is live on this COM worker; no activation parameters.
    unsafe { device.Activate(CLSCTX_ALL, None) }
}

fn endpoint_muted(device: &IMMDevice) -> Result<bool> {
    let volume = endpoint_volume(device)?;
    // SAFETY: the interface is live on this COM worker.
    unsafe { Ok(volume.GetMute()?.as_bool()) }
}

fn default_device(enumerator: &IMMDeviceEnumerator, flow: EDataFlow) -> Result<Option<IMMDevice>> {
    // SAFETY: the enumerator is live on this COM worker.
    match unsafe { enumerator.GetDefaultAudioEndpoint(flow, eMultimedia) } {
        Ok(device) => Ok(Some(device)),
        Err(error) if error.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0) => {
            log::debug!("Core Audio: no default endpoint for flow {}: {error}", flow.0);
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn devices(enumerator: &IMMDeviceEnumerator, flow: EDataFlow) -> Result<Vec<(Device, IMMDevice)>> {
    // SAFETY: the enumerator and collection stay in this initialized call.
    unsafe {
        let collection = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        let mut devices = Vec::new();
        for index in 0..collection.GetCount()? {
            let result = (|| {
                let device = collection.Item(index)?;
                let id = device_id(&device)?;
                let name = match device_name(&device) {
                    Ok(name) if !name.trim().is_empty() => name,
                    result => {
                        log::warn!("Core Audio: endpoint {id} has no readable friendly name: {result:?}");
                        id.clone()
                    }
                };
                Ok::<_, Error>((Device { id, name }, device))
            })();
            match result {
                Ok(device) => devices.push(device),
                Err(error) => log::warn!("Core Audio: reading endpoint {index}: {error}"),
            }
        }
        Ok(devices)
    }
}

fn device_id(device: &IMMDevice) -> Result<String> {
    // SAFETY: GetId returns a CoTaskMem string, freed before returning.
    unsafe { Ok(take_string(device.GetId()?)) }
}

fn device_name(device: &IMMDevice) -> Result<String> {
    // SAFETY: the property and allocated string stay local and both are freed,
    // including the property when conversion fails.
    unsafe {
        let properties = device.OpenPropertyStore(STGM_READ)?;
        let mut value = properties.GetValue(&PKEY_Device_FriendlyName)?;
        let name = PropVariantToStringAlloc(&value).map(|name| take_string(name));
        let clear = PropVariantClear(&mut value);
        match (name, clear) {
            (Ok(name), Ok(())) => Ok(name),
            (Err(error), _) | (_, Err(error)) => Err(error),
        }
    }
}

fn percent(scalar: f32) -> u16 {
    (scalar.clamp(0.0, 1.0) * 100.0).round() as u16
}

fn scalar(percent: u16) -> f32 {
    f32::from(percent.min(100)) / 100.0
}

fn stepped(current: u16, delta: i32) -> u16 {
    i32::from(current).saturating_add(delta).clamp(0, 100) as u16
}

fn not_found(message: &str) -> Error {
    Error::new(HRESULT::from_win32(ERROR_NOT_FOUND.0), message)
}

unsafe fn take_string(value: PWSTR) -> String {
    if value.is_null() {
        return String::new();
    }
    // SAFETY: callers supply only NUL-terminated strings allocated by COM.
    let string = unsafe { String::from_utf16_lossy(value.as_wide()) };
    unsafe { CoTaskMemFree(Some(value.0.cast())) };
    string
}

fn with_audio<T>(work: impl FnOnce(&IMMDeviceEnumerator) -> Result<T>) -> io::Result<T> {
    with_com(|| {
        // SAFETY: COM stays initialized on this thread until `with_com`
        // returns, after the enumerator and all of work's locals.
        let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER) }?;
        work(&enumerator)
    })
}

fn with_com<T>(work: impl FnOnce() -> Result<T>) -> io::Result<T> {
    // The multithreaded apartment when this worker has none yet; a worker
    // already in a single-threaded one keeps it. Reading endpoints and
    // sessions, setting their volume and the default device work in either
    // (only notification callbacks, which this does not register, would want
    // the MTA).
    let _com = Com::init(COINIT_MULTITHREADED);
    work().map_err(|error: Error| io::Error::other(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_volume_is_a_clamped_rounded_scalar() {
        assert_eq!(percent(0.705), 71);
        assert_eq!(percent(-0.2), 0);
        assert_eq!(percent(1.3), 100);
        assert_eq!(percent(f32::NAN), 0);
        assert_eq!(scalar(250), 1.0);
        assert_eq!(scalar(25), 0.25);
    }

    #[test]
    fn steps_handle_both_limits_and_extreme_deltas() {
        assert_eq!(stepped(70, 5), 75);
        assert_eq!(stepped(98, 5), 100);
        assert_eq!(stepped(3, -5), 0);
        assert_eq!(stepped(70, i32::MAX), 100);
        assert_eq!(stepped(70, i32::MIN), 0);
    }
}
