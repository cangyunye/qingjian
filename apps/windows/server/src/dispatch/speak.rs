//! 「朗读译文」：快捷键 → 取最近上屏语句 → 云端翻译 → SAPI 语音读出译文。
//! 与「翻译选中文字」共用 Core 的 `request_translation`，但这里没有选区可替换：
//! 候选窗只作提示（「正在翻译…」→ 译文），朗读交给 [`crate::tts`] 的专用线程，完成回执经
//! `Work::Speech` 回来收窗。最近上屏语句由壳侧攒（[`Router::note_committed`]，私密输入不记）。

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use qingjian_core::{
    Candidate, CandidateKind, CandidateList, Language, last_sentence, translation_target,
};
use qingjian_platform::protocol::{
    Frame, KeyEvent, KeyModifiers, KeyOutcome, ServerMessage, SessionId,
};

use super::Router;

/// 上屏缓冲最多留多少字符：够一条长语句，也不会无限涨。
const COMMITTED_CAP: usize = 600;

/// 翻译请求的兜底超时：超过就当失败收窗（Engine 侧轮询另有判定）。
const TRANSLATE_TIMEOUT: Duration = Duration::from_secs(12);

/// 纯提示（缓冲空 / 缺语音包）的停留时长。
const NOTICE_SECONDS: Duration = Duration::from_secs(3);

/// 发给 TTS 线程的一条命令（[`crate::tts`] 消费）。
#[derive(Debug, Clone)]
pub enum SpeechCommand {
    /// 朗读一段文本；再次 Speak 会打断当前朗读（`SPF_PURGEBEFORESPEAK`）。
    Speak { text: String, language: Language },

    /// 停止朗读。
    Stop,
}

/// TTS 线程回给 Router 的回执（经 `Work::Speech`）。
#[derive(Debug, Clone)]
pub enum SpeechNotice {
    /// 读完（或被打断后无新任务）。
    Done,

    /// 没读成：带给人看的失败原因（多为缺对应语言的语音包）。
    Failed(String),
}

/// 「朗读译文」的进行态。
pub(super) struct SpeakJob {
    /// 阶段；决定候选窗显示什么、什么时候收。
    pub(super) phase: SpeakPhase,
}

pub(super) enum SpeakPhase {
    /// 等云端译文；超时判失败。
    Translating { target: Language, deadline: Instant },

    /// 译文已交给 TTS 线程；候选窗显示译文，等回执收窗。
    Speaking { text: String },

    /// 纯提示；到点自动收窗。
    Notice { text: String, deadline: Instant },
}

impl Router {
    /// 「朗读译文」的组合键（与 [`Router::matches_translate_combo`](super::translate) 同型）。
    pub(super) fn matches_speak_combo(&self, event: &KeyEvent) -> bool {
        let combo = self.config.speak_translation;
        event.character == Some(combo.key)
            && event.modifiers.chord() == KeyModifiers::from(combo.modifiers)
    }

    /// 触发朗读：打断上一段，取最近上屏语句发翻译请求，候选窗显示「正在翻译…」。
    pub(super) fn start_speak(&mut self, session: SessionId) -> ServerMessage {
        if self.speak.is_some() {
            self.stop_speech_audio();
        }
        self.speak = None;
        let Some(sentence) = self.last_committed_sentence() else {
            tracing::info!("朗读译文：还没有可用的上屏语句");
            return self.speak_notice(session, "还没有上屏的内容");
        };
        let target = translation_target(&sentence, self.engine.learning_language());
        self.engine.request_translation(&sentence);
        tracing::debug!(
            chars = sentence.chars().count(),
            ?target,
            "朗读译文：已发翻译请求"
        );
        self.speak = Some(SpeakJob {
            phase: SpeakPhase::Translating {
                target,
                deadline: Instant::now() + TRANSLATE_TIMEOUT,
            },
        });
        let frame = self.speak_frame("正在翻译…");
        self.show_speak_frame(frame);
        ServerMessage::KeyResult {
            session,
            outcome: KeyOutcome::Consumed,
            commit: None,
            frame: Frame::default(),
        }
    }

    /// 最近一条上屏语句：从缓冲取，超长截尾（与「翻译选中文字」的 500 字上限对齐）。
    fn last_committed_sentence(&self) -> Option<String> {
        let sentence = last_sentence(&self.committed)?;
        let skip = sentence.chars().count().saturating_sub(500);
        Some(sentence.chars().skip(skip).collect())
    }

    /// 记一段上屏文本（私密输入不记），超出容量从头丢。
    pub(super) fn note_committed(&mut self, text: &str) {
        if text.is_empty() || self.is_private() {
            return;
        }
        self.committed.push_str(text);
        let count = self.committed.chars().count();
        if count > COMMITTED_CAP {
            let drop = count - COMMITTED_CAP;
            let start = self
                .committed
                .char_indices()
                .nth(drop)
                .map_or(self.committed.len(), |(i, _)| i);
            self.committed.drain(..start);
        }
    }

    /// 云端译文到了：交给 TTS 线程朗读，候选窗换成译文等回执。
    pub(super) fn begin_speaking(&mut self, text: String) {
        let Some(job) = self.speak.as_mut() else {
            return;
        };
        let SpeakPhase::Translating { target, .. } = job.phase else {
            return;
        };
        match self.speech.as_ref() {
            Some(sender) => {
                let _ = sender.send(SpeechCommand::Speak {
                    text: text.clone(),
                    language: target,
                });
                if let Some(job) = self.speak.as_mut() {
                    job.phase = SpeakPhase::Speaking { text };
                }
                let frame = self
                    .current_speak_text()
                    .map_or(Frame::default(), |t| self.speak_frame(&t));
                self.show_speak_frame(frame);
            }
            None => {
                tracing::warn!("没有语音合成线程（非 Windows 构建？），只显示译文");
                self.speak_notice_in_place(text);
            }
        }
    }

    /// 提示帧：不经过翻译，直接显示一段文字几秒。
    pub(super) fn speak_notice(&mut self, session: SessionId, text: &str) -> ServerMessage {
        self.speak_notice_in_place(text.to_owned());
        ServerMessage::KeyResult {
            session,
            outcome: KeyOutcome::Consumed,
            commit: None,
            frame: Frame::default(),
        }
    }

    pub(super) fn speak_notice_in_place(&mut self, text: String) {
        let frame = self.speak_frame(&text);
        self.speak = Some(SpeakJob {
            phase: SpeakPhase::Notice {
                text,
                deadline: Instant::now() + NOTICE_SECONDS,
            },
        });
        self.show_speak_frame(frame);
    }

    /// 当前该显示的文本；没在朗读态为 `None`。
    pub(super) fn current_speak_text(&self) -> Option<String> {
        match &self.speak {
            Some(job) => match &job.phase {
                SpeakPhase::Speaking { text } => Some(text.clone()),
                SpeakPhase::Notice { text, .. } => Some(text.clone()),
                SpeakPhase::Translating { .. } => Some("正在翻译…".to_owned()),
            },
            None => None,
        }
    }

    /// 朗读进行态的帧：单条候选，无 preedit（与「翻译中…」评审帧同型）。
    pub(super) fn speak_frame(&self, text: &str) -> Frame {
        Frame {
            preedit: Vec::new(),
            cursor: 0,
            candidates: CandidateList {
                items: vec![Candidate {
                    text: text.to_owned(),
                    kind: CandidateKind::Cloud,
                    syllables: Vec::new(),
                    reading: None,
                    translation: None,
                }],
            },
            highlight: 0,
            page: 0,
            page_count: 1,
            layout: self.config.layout,
            theme: self.config.theme,
            sentence: None,
            notice: None,
        }
    }

    /// 把朗读帧摆在最近一次组句的光标位置：上屏文本的落点。组句结束后的位置上报已停，
    /// 所以 `position_candidates` 另存了一份 [`Router::last_caret`] 从不清。
    fn show_speak_frame(&mut self, frame: Frame) {
        if let Some(rect) = self.last_caret {
            self.last_rect = Some(rect);
            self.reconcile_candidates(&frame);
        }
    }

    /// 任何其它按键：结束朗读流程（清帧、停音频、作废在飞的翻译）。
    pub(super) fn end_speak(&mut self) {
        if let Some(job) = self.speak.take() {
            // 只有真的起播过才需要停音频；纯提示没有声音。
            if matches!(job.phase, SpeakPhase::Speaking { .. }) {
                self.stop_speech_audio();
            }
            self.cancel_prediction();
            self.hide_candidate_window();
        }
    }

    /// TTS 线程回执：读完收窗；失败换提示帧。
    pub fn handle_speech_notice(&mut self, notice: SpeechNotice) {
        match notice {
            SpeechNotice::Done => {
                if self.speak.take().is_some() {
                    self.hide_candidate_window();
                }
            }
            SpeechNotice::Failed(message) => {
                tracing::warn!(%message, "朗读译文失败");
                if self.speak.is_some() {
                    self.speak_notice_in_place(message);
                }
            }
        }
    }

    /// 停掉音频；没有 TTS 线程时是空操作。
    fn stop_speech_audio(&mut self) {
        if let Some(sender) = self.speech.as_ref() {
            let _ = sender.send(SpeechCommand::Stop);
        }
    }

    /// tick 里看一眼：等译文超时、提示到点，都收窗。
    pub(super) fn poll_speak_deadline(&mut self) {
        let expired = match &self.speak {
            Some(job) => match &job.phase {
                SpeakPhase::Translating { deadline, .. } => *deadline < Instant::now(),
                SpeakPhase::Notice { deadline, .. } => *deadline < Instant::now(),
                SpeakPhase::Speaking { .. } => false,
            },
            None => false,
        };
        if expired {
            tracing::info!("朗读译文：等待超时，收窗");
            self.end_speak();
        }
    }

    /// TTS 线程的命令端；非 Windows 构建（测试）不设。
    pub fn set_speech(&mut self, sender: Sender<SpeechCommand>) {
        self.speech = Some(sender);
    }
}
