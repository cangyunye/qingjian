use std::sync::mpsc::Sender;

use qingjian_platform::protocol::{ClientMessage, ServerMessage};

use crate::dispatch::{SpeechNotice, StatusEvent};

/// 工人线程（独占 [`crate::dispatch::Router`]）的一件活：DLL 的一条消息、状态条上的一次操作，
/// 或 TTS 线程的一次朗读回执。
pub enum Work {
    /// 某条连接收到的消息 + 回结果的通道（`None` 表示不用回话）。
    Client(ClientMessage, Sender<Option<ServerMessage>>),

    /// UI 线程发来的状态条操作。
    Status(StatusEvent),

    /// TTS 线程的朗读回执（读完 / 失败）。
    Speech(SpeechNotice),
}
