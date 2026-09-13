//! SAPI 朗读冒烟测试（仅 Windows，默认 `#[ignore]`：真的会出声）。手跑：`task tts-smoke`。
//! 没装对应语音包时断言失败信息里会带「没有找到…语音包」，方便区分「合成坏了」与「缺包」。

#![cfg(windows)]

use std::time::Duration;

use qingjian_core::Language;
use qingjian_windows_server::dispatch::{SpeechCommand, SpeechNotice};
use qingjian_windows_server::ipc::Work;
use qingjian_windows_server::tts;

/// 英语一句，等读完的回执（最多 30 秒）。
#[test]
#[ignore = "真的会出声，手动验证时跑"]
fn sapi_speaks_english_and_reports_done() {
    let (work_tx, work_rx) = std::sync::mpsc::channel();
    let commands = tts::spawn(work_tx);
    commands
        .send(SpeechCommand::Speak {
            text: "Hello, this is a voice test.".to_owned(),
            language: Language::English,
        })
        .expect("发朗读命令");
    let mut notice = None;
    for _ in 0..300 {
        match work_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Work::Speech(result)) => {
                notice = Some(result);
                break;
            }
            Ok(_) => {}
            // 超时=还在朗读，继续等；通道断开才是线程提前退出。
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
    }
    let notice = notice.expect("30 秒内应有朗读回执");
    assert!(
        matches!(notice, SpeechNotice::Done),
        "应正常读完，实际: {notice:?}（缺语音包请看提示装包）"
    );
}
