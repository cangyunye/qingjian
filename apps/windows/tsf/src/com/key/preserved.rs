//! 「翻译选中文字」「朗读译文」的快捷键登记成 TSF **保留键**（preserved key）。带 Alt 的组合是系统键，不经击键 sink
//! （真机：Ctrl+Alt+T 在 `OnTestKeyDown` 里从没出现过）；保留键由 TSF 在应用之前匹配、回调 `OnPreservedKey`，
//! UWP 里也一样。组合来自 `[shortcut]`，激活时读一次配置（AppContainer 读不到用户目录时用缺省）；
//! 改了配置要切走再切回输入法才重新登记。

use std::path::PathBuf;

use windows::Win32::UI::TextServices::{
    ITfKeystrokeMgr, TF_MOD_ALT, TF_MOD_CONTROL, TF_MOD_SHIFT, TF_PRESERVEDKEY,
};
use windows::core::{GUID, Result};

use qingjian_platform::protocol::{KeyEvent, KeyModifiers};
use qingjian_platform::{Config, KeyCombo};

use crate::com::log::log;

/// 「翻译选中文字」保留键的标识，`OnPreservedKey` 按它认。
pub(crate) const GUID_TRANSLATE: GUID = GUID::from_u128(0x5c0a7b12_3d4e_4f60_8a91_2b3c4d5e6f70);

/// 「朗读译文」保留键的标识。
pub(crate) const GUID_SPEAK: GUID = GUID::from_u128(0x7d3c8f24_5e61_4a73_9b02_3c4d5e6f7182);

/// msctf.h 的 `TF_MOD_LWIN`（windows crate 没导出）。
const TF_MOD_LWIN: u32 = 0x08;

/// 读 `%APPDATA%\Qingjian\config.toml`；读不到 / 解析失败记日志并返回 `None`。
fn read_config() -> Option<Config> {
    let path = std::env::var_os("APPDATA")
        .map(|base| PathBuf::from(base).join("Qingjian").join("config.toml"))?;
    match Config::load(&path) {
        Ok(config) => Some(config),
        Err(error) => {
            log(&format!("读配置取快捷键失败，用缺省: {error}"));
            None
        }
    }
}

/// 读「翻译选中文字」的组合；读不到用缺省。
pub(crate) fn load_combo() -> KeyCombo {
    read_config()
        .map(|config| config.shortcut.translate_selection)
        .unwrap_or(KeyCombo::TRANSLATE_DEFAULT)
}

/// 读「朗读译文」的组合；与「翻译选中文字」撞键时 `speak_keys()` 已退回缺省，与 Server 侧一致。
pub(crate) fn load_speak_combo() -> KeyCombo {
    read_config()
        .map(|config| config.shortcut.speak_keys())
        .unwrap_or(KeyCombo::SPEAK_DEFAULT)
}

fn preserved_key(combo: KeyCombo) -> TF_PRESERVEDKEY {
    let m = combo.modifiers;
    let mut modifiers = 0;
    if m.control {
        modifiers |= TF_MOD_CONTROL;
    }
    if m.option {
        modifiers |= TF_MOD_ALT;
    }
    if m.shift {
        modifiers |= TF_MOD_SHIFT;
    }
    if m.command {
        modifiers |= TF_MOD_LWIN;
    }
    TF_PRESERVEDKEY {
        uVKey: combo.key.to_ascii_uppercase() as u32,
        uModifiers: modifiers,
    }
}

pub(crate) fn register(
    keystroke: &ITfKeystrokeMgr,
    tid: u32,
    guid: &GUID,
    combo: KeyCombo,
    description: &str,
) -> Result<()> {
    let key = preserved_key(combo);
    let description: Vec<u16> = description.encode_utf16().collect();
    unsafe { keystroke.PreserveKey(tid, guid, &key, &description) }
}

pub(crate) fn unregister(keystroke: &ITfKeystrokeMgr, guid: &GUID, combo: KeyCombo) {
    let key = preserved_key(combo);
    let _ = unsafe { keystroke.UnpreserveKey(guid, &key) };
}

/// 保留键命中时喂给 Server 的按键：Router 按字符 + 物理修饰键与配置比对。
pub(crate) fn key_event(combo: KeyCombo, english_mode: bool) -> KeyEvent {
    let m = combo.modifiers;
    KeyEvent::new(
        combo.key.to_ascii_uppercase() as u32,
        Some(combo.key),
        KeyModifiers {
            ctrl: m.control,
            shift: m.shift,
            alt: m.option,
            win: m.command,
            caps: false,
            english_mode,
        },
    )
}
