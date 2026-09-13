//! 本地输入历史：本次会话里经我们上屏的文本，按时间顺序拼成一段。
//!
//! 只用于两件事：应用不提供光标附近文本时给联想当上下文；以后给个人模型当训练数据。
//! 全部在内存里，落盘由壳决定；容量有上限，超过就丢最旧的。

/// 内存里最多保留多少个字符。够联想的观察窗口用，也不会无限增长。
const DEFAULT_CAPACITY: usize = 4096;

/// 「朗读译文」取最近一条语句时认的句末标点。
const SENTENCE_ENDS: [char; 7] = ['。', '！', '？', '；', '…', '!', '?'];

/// 从上屏拼接文本里取最近一条语句：去掉尾部的句末标点与空白，
/// 再从最后一个句末标点之后取剩下的一段；没有句末标点就整段算一条。
/// 英文句点只在后跟空白或结尾时算句末，免得切开 3.5、a.cc 这类写法。
pub fn last_sentence(text: &str) -> Option<&str> {
    let trimmed = text
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace() || SENTENCE_ENDS.contains(&c));
    let mut last_end = None;
    let mut chars = trimmed.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let ends = match c {
            '.' => chars.peek().is_none_or(|&(_, next)| next.is_whitespace()),
            '\n' => true,
            c => SENTENCE_ENDS.contains(&c),
        };
        if ends {
            last_end = Some(i + c.len_utf8());
        }
    }
    match last_end {
        Some(start) => trimmed.get(start..),
        None if trimmed.is_empty() => None,
        None => Some(trimmed),
    }
}

#[derive(Debug, Clone)]
pub struct InputHistory {
    /// 已上屏文本，按时间顺序首尾相接。
    text: String,

    /// 字符数上限。
    capacity: usize,
}

impl Default for InputHistory {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
}

impl InputHistory {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            text: String::new(),
            capacity,
        }
    }

    /// 追加一段上屏文本，超出容量时从头丢弃。
    pub fn record(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.text.push_str(text);
        let count = self.text.chars().count();
        if count > self.capacity {
            let drop = count - self.capacity;
            let start = self
                .text
                .char_indices()
                .nth(drop)
                .map_or(self.text.len(), |(i, _)| i);
            self.text.drain(..start);
        }
    }

    /// 最近 `chars` 个字符。
    pub fn recent(&self, chars: usize) -> &str {
        if chars == 0 {
            return "";
        }
        let count = self.text.chars().count();
        if chars >= count {
            return &self.text;
        }
        let start = self
            .text
            .char_indices()
            .nth(count - chars)
            .map_or(0, |(i, _)| i);
        &self.text[start..]
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// 最近一条上屏语句，供壳「朗读译文」用。
    pub fn last_sentence(&self) -> Option<&str> {
        last_sentence(&self.text)
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// 一键清除。
    pub fn clear(&mut self) {
        self.text.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_in_order_and_returns_recent_chars() {
        let mut history = InputHistory::default();
        history.record("我们");
        history.record("今天");
        history.record("");
        assert_eq!(history.text(), "我们今天");
        assert_eq!(history.recent(2), "今天");
        assert_eq!(history.recent(10), "我们今天");
        assert_eq!(history.recent(0), "");
    }

    #[test]
    fn drops_oldest_beyond_capacity_on_char_boundaries() {
        let mut history = InputHistory::with_capacity(3);
        history.record("开发");
        history.record("输入法");
        assert_eq!(history.text(), "输入法");
        history.record("a");
        assert_eq!(history.text(), "入法a");
        history.clear();
        assert!(history.is_empty());
    }

    #[test]
    fn last_sentence_takes_text_after_latest_ender() {
        assert_eq!(
            last_sentence("你好世界。今天天气不错"),
            Some("今天天气不错")
        );
        assert_eq!(last_sentence("你好。今天！"), Some("今天"));
        assert_eq!(last_sentence("第一句。第二句；第三句"), Some("第三句"));
    }

    #[test]
    fn last_sentence_without_ender_is_whole_text() {
        assert_eq!(last_sentence("你好"), Some("你好"));
        assert_eq!(last_sentence("你好。"), Some("你好"));
        assert_eq!(last_sentence("价格3.5元"), Some("价格3.5元"));
        assert_eq!(last_sentence("a.cc 也是路径"), Some("a.cc 也是路径"));
    }

    #[test]
    fn last_sentence_empty_or_pure_punctuation_is_none() {
        assert_eq!(last_sentence(""), None);
        assert_eq!(last_sentence("。。。"), None);
        assert_eq!(last_sentence("  \n "), None);
        assert_eq!(last_sentence("\n换行后的"), Some("换行后的"));
    }

    #[test]
    fn history_last_sentence_reads_recorded_text() {
        let mut history = InputHistory::default();
        history.record("早上好。");
        assert_eq!(history.last_sentence(), Some("早上好"));
        history.record("现在呢");
        assert_eq!(history.last_sentence(), Some("现在呢"));
    }
}
