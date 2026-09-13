//! SAPI 语音合成线程（仅 Windows）：持有 `ISpVoice`，消费 [`SpeechCommand`]，回执经 [`Work::Speech`]
//! 送回 Router 收窗。朗读走 `SPF_ASYNC` 起播，每 100 ms 用 `GetStatus` 的 `SPRS_IS_SPEAKING` 位查完成；
//! 新的 `Speak` 带 `SPF_PURGEBEFORESPEAK` 直接打断当前朗读。音色按目标语言从 `SPCAT_VOICES` 类目里挑
//! （`Language=<LCID 十六进制>` 属性），找不到就回执失败——大多是系统没装那个语言的语音包。

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use qingjian_core::Language;
use windows::Win32::Media::Speech::{
    ISpObjectToken, ISpObjectTokenCategory, ISpVoice, SPF_ASYNC, SPF_PURGEBEFORESPEAK,
    SPRS_IS_SPEAKING, SPVOICESTATUS, SpObjectTokenCategory, SpVoice,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::core::{HSTRING, PWSTR};

use crate::dispatch::{SpeechCommand, SpeechNotice};
use crate::ipc::Work;

/// 完成检测的间隔。
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// SAPI 语音类目（sapi.h 的 `SPCAT_VOICES`，windows crate 里是 PCWSTR，转个手写常量）。
const VOICES_CATEGORY: &str = "HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Speech\\Voices";

/// 起 TTS 线程；返回命令发送端。线程随进程存活（通道关闭即退出）。
pub fn spawn(work_tx: Sender<Work>) -> Sender<SpeechCommand> {
    let (command_tx, command_rx) = std::sync::mpsc::channel::<SpeechCommand>();
    thread::Builder::new()
        .name("qingjian-tts".to_owned())
        .spawn(move || run(command_rx, work_tx))
        .expect("起语音合成线程");
    command_tx
}

fn run(rx: Receiver<SpeechCommand>, work_tx: Sender<Work>) {
    // Server 全进程此前没有 COM；本线程自管 apartment（MTA，跨线程调用无需消息泵）。
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if initialized.is_err() {
        tracing::warn!(hr = %initialized, "语音合成初始化失败");
        return;
    }
    let voice: ISpVoice = match unsafe { CoCreateInstance(&SpVoice, None, CLSCTX_ALL) } {
        Ok(voice) => voice,
        Err(error) => {
            tracing::warn!(%error, "创建 ISpVoice 失败");
            unsafe { CoUninitialize() };
            return;
        }
    };
    // 通道关闭（进程退出）即收尾。
    while let Ok(command) = rx.recv() {
        if let SpeechCommand::Speak { text, language } = command {
            speak_one(&voice, &rx, &work_tx, text, language);
        }
    }
    unsafe { CoUninitialize() };
}

/// 朗读一段直到读完或被打断；打断时的下一条命令由外层循环接手。
fn speak_one(
    voice: &ISpVoice,
    rx: &Receiver<SpeechCommand>,
    work_tx: &Sender<Work>,
    text: String,
    language: Language,
) {
    if let Err(message) = select_voice(voice, language) {
        let _ = work_tx.send(Work::Speech(SpeechNotice::Failed(message)));
        return;
    }
    let wide = HSTRING::from(text.as_str());
    let flags = (SPF_ASYNC.0 | SPF_PURGEBEFORESPEAK.0) as u32;
    if let Err(error) = unsafe { voice.Speak(&wide, flags, None) } {
        tracing::warn!(%error, "SAPI Speak 失败");
        let _ = work_tx.send(Work::Speech(SpeechNotice::Failed(format!(
            "朗读失败: {error}"
        ))));
        return;
    }
    loop {
        match rx.recv_timeout(POLL_INTERVAL) {
            // 新命令留给外层循环接手；这里先把当前朗读清掉（新的 Speak 自己带 PURGE）。
            Ok(SpeechCommand::Speak { .. }) | Ok(SpeechCommand::Stop) => {
                let _ =
                    unsafe { voice.Speak(&HSTRING::new(), SPF_PURGEBEFORESPEAK.0 as u32, None) };
                return;
            }
            Err(RecvTimeoutError::Timeout) => {
                let mut status = SPVOICESTATUS::default();
                let mut bookmark = PWSTR::null();
                match unsafe { voice.GetStatus(&mut status, &mut bookmark) } {
                    Ok(()) if status.dwRunningState & SPRS_IS_SPEAKING.0 as u32 == 0 => {
                        let _ = work_tx.send(Work::Speech(SpeechNotice::Done));
                        return;
                    }
                    Ok(()) => {}
                    Err(error) => {
                        tracing::warn!(%error, "查朗读状态失败");
                        let _ = work_tx.send(Work::Speech(SpeechNotice::Done));
                        return;
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// 按目标语言挑音色；找不到对应语音包就报错（缺包是最常见的失败）。
fn select_voice(voice: &ISpVoice, language: Language) -> Result<(), String> {
    let (attribute, name) = match language {
        Language::Chinese => ("Language=804", "中文"),
        Language::English => ("Language=409", "英语"),
        Language::Japanese => ("Language=411", "日语"),
    };
    let category: ISpObjectTokenCategory =
        unsafe { CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_ALL) }
            .map_err(|error| format!("语音合成不可用: {error}"))?;
    unsafe { category.SetId(&HSTRING::from(VOICES_CATEGORY), false) }
        .map_err(|error| format!("语音合成不可用: {error}"))?;
    let tokens = unsafe { category.EnumTokens(&HSTRING::from(attribute), &HSTRING::new()) }
        .map_err(|error| format!("没有找到{name}的语音包（枚举失败: {error}）"))?;
    let mut token: Option<ISpObjectToken> = None;
    let mut fetched: u32 = 0;
    unsafe { tokens.Next(1, &mut token, Some(&mut fetched)) }
        .map_err(|error| format!("没有找到{name}的语音包（枚举失败: {error}"))?;
    if fetched == 0 || token.is_none() {
        return Err(format!(
            "没有找到{name}的语音包，请在系统设置 → 时间和语言 → 语音里添加"
        ));
    }
    unsafe { voice.SetVoice(&token.unwrap()) }
        .map_err(|error| format!("切换{name}音色失败: {error}"))
}
