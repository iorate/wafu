use std::sync::{Arc, Weak};
use std::thread;

use wafu_core::{ColorScheme, IoEvent};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, KEY_QUERY_VALUE, REG_NOTIFY_CHANGE_LAST_SET,
    RRF_RT_REG_DWORD, RegCloseKey, RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW,
};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};
use windows::core::{PCWSTR, w};

use crate::runtime::OnEvent;

// Dropping it ends the thread once the key changes again.
pub(crate) struct ColorSchemeWatch {
    _alive: Arc<()>,
}

// winit decides the scheme by calling uxtheme's `ShouldAppsUseDarkMode` when a
// `WM_SETTINGCHANGE` arrives, and that function may still return the old
// value at that moment. Switchers that broadcast only once (PowerToys Light
// Switch) then leave the window on the old scheme until an unrelated setting
// changes, so the registry value every switcher writes is watched instead.
pub(crate) fn watch_color_scheme(on_event: &OnEvent) -> ColorSchemeWatch {
    let alive = Arc::new(());
    let weak = Arc::downgrade(&alive);
    let report = Arc::clone(on_event);
    thread::Builder::new()
        .name("wafu-io-color-scheme".to_owned())
        .spawn(move || watch(&report, &weak))
        .expect("the color scheme thread should start");
    ColorSchemeWatch { _alive: alive }
}

#[allow(unsafe_code)]
fn watch(report: &OnEvent, alive: &Weak<()>) {
    let mut key = HKEY::default();
    // SAFETY: the subkey is a wide string literal and `key` is writable.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            None,
            KEY_QUERY_VALUE | KEY_NOTIFY,
            &raw mut key,
        )
    };
    if opened.is_err() {
        return;
    }
    // SAFETY: an auto-reset event without a name needs no attributes.
    let Ok(event) = (unsafe { CreateEventW(None, false, false, PCWSTR::null()) }) else {
        // SAFETY: the key was opened above and is not used afterwards.
        let _ = unsafe { RegCloseKey(key) };
        return;
    };
    let mut last = None;
    loop {
        // Armed before the value is read, so that a change between the read
        // and the wait is not missed.
        // SAFETY: the key is open for notification and the event stays open
        // until the thread ends.
        let armed = unsafe {
            RegNotifyChangeKeyValue(key, false, REG_NOTIFY_CHANGE_LAST_SET, Some(event), true)
        };
        if armed.is_err() {
            break;
        }
        let scheme = read_scheme(key);
        if last != Some(scheme) {
            last = Some(scheme);
            report(IoEvent::ColorScheme { scheme });
        }
        // SAFETY: the event handle is open.
        unsafe { WaitForSingleObject(event, INFINITE) };
        if alive.upgrade().is_none() {
            break;
        }
    }
    // SAFETY: both handles were opened above and are not used afterwards.
    unsafe {
        let _ = CloseHandle(event);
        let _ = RegCloseKey(key);
    }
}

#[allow(unsafe_code)]
fn read_scheme(key: HKEY) -> ColorScheme {
    let mut light: u32 = 1;
    let mut size = u32::try_from(size_of::<u32>()).unwrap_or(0);
    // SAFETY: the key is open for reading, and `light` is a DWORD-sized
    // buffer whose size is passed with it.
    let read = unsafe {
        RegGetValueW(
            key,
            PCWSTR::null(),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut light).cast()),
            Some(&raw mut size),
        )
    };
    // Without the value, Windows uses the light scheme.
    if read.is_ok() && light == 0 {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    }
}
