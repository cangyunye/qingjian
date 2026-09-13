use qingjian_core::ModeKeys;
use serde::{Deserialize, Serialize};

use super::key_combo::KeyCombo;
use super::modifiers::Modifiers;

/// 配置文件 `[shortcut]` 分节：前缀模式键（Core 的 [`ModeKeys`]）加壳层的修饰键组合。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutConfig {
    /// 表达式 / 问字模式键，键名与以前一样直接在分节下（`expression` / `question`）。
    #[serde(flatten)]
    pub mode: ModeKeys,

    /// 数字键配这些修饰键：上屏候选的第一个译词。
    pub translation: Modifiers,

    /// 数字键配这些修饰键：上屏候选的第二个译词（候选右侧有两个译词时）。
    pub translation_second: Modifiers,

    /// 把应用里选中的文字译成学习语言（需要云服务开着）。
    pub translate_selection: KeyCombo,

    /// 朗读最近上屏语句的译文（需要云服务开着；译文用系统本地语音读出来）。
    pub speak_translation: KeyCombo,

    /// 数字键配这些修饰键：删掉候选（用户词整个删掉，词库词清掉对它的学习）。
    pub delete_candidate: Modifiers,
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        // Windows 上 Alt+数字被系统当菜单快捷键截走（TSF 收不到），译词键缺省用 Ctrl；macOS 用 Option。
        #[cfg(windows)]
        let (translation, translation_second) = (Modifiers::CONTROL, Modifiers::SHIFT_CONTROL);
        #[cfg(not(windows))]
        let (translation, translation_second) = (Modifiers::OPTION, Modifiers::SHIFT_OPTION);
        Self {
            mode: ModeKeys::default(),
            translation,
            translation_second,
            translate_selection: KeyCombo::TRANSLATE_DEFAULT,
            speak_translation: KeyCombo::SPEAK_DEFAULT,
            delete_candidate: Modifiers::SHIFT,
        }
    }
}

impl ShortcutConfig {
    /// 「朗读译文」的组合键；与「翻译选中文字」撞了就退回缺省，保证两个功能都可用。
    pub fn speak_keys(&self) -> KeyCombo {
        if self.speak_translation == self.translate_selection {
            Self::default().speak_translation
        } else {
            self.speak_translation
        }
    }

    /// 删候选的修饰键；为空或与任一组译词键撞了就退回缺省。
    pub fn delete_keys(&self) -> Modifiers {
        let (first, second) = self.translation_keys();
        if self.delete_candidate.is_empty()
            || self.delete_candidate == first
            || self.delete_candidate == second
        {
            Self::default().delete_candidate
        } else {
            self.delete_candidate
        }
    }

    /// 两组译词修饰键；两组相同或有一组为空时整个退回缺省，不做一半。
    pub fn translation_keys(&self) -> (Modifiers, Modifiers) {
        if self.translation == self.translation_second
            || self.translation.is_empty()
            || self.translation_second.is_empty()
        {
            let default = Self::default();
            (default.translation, default.translation_second)
        } else {
            (self.translation, self.translation_second)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_files_without_modifier_keys_still_parse_and_get_defaults() {
        let parsed: ShortcutConfig = toml::from_str("expression = \"i\"\n").unwrap();
        assert_eq!(parsed.mode.expression, 'i');
        assert_eq!(
            parsed.translation_keys(),
            ShortcutConfig::default().translation_keys()
        );
        let same: ShortcutConfig =
            toml::from_str("translation = \"option\"\ntranslation_second = \"option\"\n").unwrap();
        assert_eq!(
            same.translation_keys(),
            ShortcutConfig::default().translation_keys()
        );
        let swapped: ShortcutConfig =
            toml::from_str("translation = \"control+option\"\ntranslation_second = \"option\"\n")
                .unwrap();
        assert_eq!(swapped.translation_keys().1, Modifiers::OPTION);
    }

    #[test]
    fn delete_keys_fall_back_when_clashing_with_translation_keys() {
        let parsed: ShortcutConfig = toml::from_str("").unwrap();
        assert_eq!(parsed.delete_keys(), Modifiers::SHIFT);
        // 与当前平台的第一组译词键撞上才退回缺省
        let first = ShortcutConfig::default().translation_keys().0.key();
        let clash: ShortcutConfig =
            toml::from_str(&format!("delete_candidate = \"{first}\"\n")).unwrap();
        assert_eq!(clash.delete_keys(), Modifiers::SHIFT);
        let custom: ShortcutConfig =
            toml::from_str("delete_candidate = \"control+option\"\n").unwrap();
        assert!(custom.delete_keys().control);
    }

    #[test]
    fn speak_keys_parse_and_fall_back_when_clashing_with_translate() {
        let parsed: ShortcutConfig = toml::from_str("").unwrap();
        assert_eq!(parsed.speak_keys(), KeyCombo::SPEAK_DEFAULT);
        let custom: ShortcutConfig =
            toml::from_str("speak_translation = \"control+option+s\"\n").unwrap();
        assert_eq!(custom.speak_keys().key, 's');
        let clash: ShortcutConfig =
            toml::from_str("speak_translation = \"control+option+t\"\n").unwrap();
        assert_eq!(clash.speak_keys(), KeyCombo::SPEAK_DEFAULT);
    }
}
