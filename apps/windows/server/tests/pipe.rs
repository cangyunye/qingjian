//! 真命名管道的端到端测试（仅 Windows）：监听线程 + 文件句柄客户端，走完「开会话 → 敲 nihao → 收候选」。

#![cfg(windows)]

use std::fs::OpenOptions;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use qingjian_core::Language;
use qingjian_core::{Prediction, PredictionKind, PredictionPolicy, PredictionRequest, Predictor};
use qingjian_platform::protocol::{
    ClientMessage, KeyEvent, KeyModifiers, PROTOCOL_VERSION, ServerMessage, SessionId,
};
use qingjian_windows_server::dispatch::SpeechCommand;
use qingjian_windows_server::ipc::pipe::serve_pipe;
use qingjian_windows_server::ipc::{read_message, write_message};
use qingjian_windows_server::{AssemblySpec, Router, RouterConfig, assembly};

const SESSION: SessionId = SessionId(1);

fn letter(c: char) -> KeyEvent {
    KeyEvent::new(c.to_ascii_uppercase() as u32, Some(c), Default::default())
}

/// 客户端重试打开管道，直到监听线程建好实例。
fn connect(name: &str) -> std::fs::File {
    for _ in 0..50 {
        match OpenOptions::new().read(true).write(true).open(name) {
            Ok(file) => return file,
            Err(_) => thread::sleep(Duration::from_millis(50)),
        }
    }
    panic!("连不上管道 {name}");
}

#[test]
fn named_pipe_round_trips_the_open_type_loop() {
    let name = format!(r"\\.\pipe\qingjian-test-{}", std::process::id());

    // 监听线程服务完一个客户端后阻塞等下一个，随进程退出即可。
    let server_name = name.clone();
    thread::spawn(move || {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let engine = assembly::assemble(&AssemblySpec {
            glossary: Some((
                Language::English,
                root.join("assets/sample/glossary-en.tsv"),
            )),
            ..AssemblySpec::new(root.join("assets/sample/dict.tsv"))
        })
        .expect("assemble engine from sample data");
        let mut router = Router::new(engine, RouterConfig::default());
        let (work_tx, work_rx) = std::sync::mpsc::channel();
        let _ = serve_pipe(&server_name, &mut router, work_tx, work_rx);
    });

    let mut client = connect(&name);

    write_message(
        &mut client,
        &ClientMessage::OpenSession {
            session: SESSION,
            app: None,
            protocol: PROTOCOL_VERSION,
        },
    )
    .unwrap();
    for c in "nihao".chars() {
        write_message(
            &mut client,
            &ClientMessage::Key {
                session: SESSION,
                event: letter(c),
            },
        )
        .unwrap();
    }

    // 开会话先回一条 `SessionOpened`，之后五个按键各回一条 `KeyResult`。
    let mut last_frame = None;
    for _ in 0..6 {
        let message: ServerMessage = read_message(&mut client)
            .expect("read response")
            .expect("server closed early");
        if let ServerMessage::KeyResult { frame, .. } = message {
            last_frame = Some(frame);
        }
    }

    let frame = last_frame.expect("至少一条 KeyResult");
    let preedit: String = frame.preedit.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(preedit, "ni'hao");
    let texts: Vec<&str> = frame
        .candidates
        .items
        .iter()
        .map(|c| c.text.as_str())
        .collect();
    assert!(
        texts.contains(&"你好"),
        "候选里应有「你好」，实际：{texts:?}"
    );
}

/// 测试用联想器：Translate 请求回一句固定译文，其余不答。
struct MockPredictor {
    reply: Option<Prediction>,
}

impl Predictor for MockPredictor {
    fn policy(&self) -> PredictionPolicy {
        PredictionPolicy::default()
    }

    fn submit(&mut self, request: PredictionRequest) {
        if matches!(request.kind, PredictionKind::Translate) {
            self.reply = Some(Prediction {
                sequence: request.sequence,
                words: Vec::new(),
                sentence: Some("Hello world".to_owned()),
            });
        }
    }

    fn poll(&mut self) -> Option<Prediction> {
        self.reply.take()
    }

    fn is_enabled(&self) -> bool {
        true
    }
}

/// 「朗读译文」的缺省组合（ctrl+alt+r）。
fn speak_combo() -> KeyEvent {
    KeyEvent::new(
        'R' as u32,
        Some('r'),
        KeyModifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        },
    )
}

fn space() -> KeyEvent {
    KeyEvent::new(0x20, Some(' '), Default::default())
}

#[test]
fn speak_last_committed_sentence_translates_and_hands_to_tts() {
    let name = format!(r"\\.\pipe\qingjian-test-speak-{}", std::process::id());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let engine = assembly::assemble(&AssemblySpec {
        glossary: Some((
            Language::English,
            root.join("assets/sample/glossary-en.tsv"),
        )),
        ..AssemblySpec::new(root.join("assets/sample/dict.tsv"))
    })
    .expect("assemble engine from sample data");
    let mut engine = engine;
    engine.set_predictor(Box::new(MockPredictor { reply: None }));
    let mut router = Router::new(engine, RouterConfig::default());
    // 译文交出去的命令由测试收着看；TTS 线程本身不起（不起音频）。
    let (speech_tx, speech_rx) = std::sync::mpsc::channel();
    router.set_speech(speech_tx);
    let server_name = name.clone();
    thread::spawn(move || {
        let (work_tx, work_rx) = std::sync::mpsc::channel();
        let _ = serve_pipe(&server_name, &mut router, work_tx, work_rx);
    });

    let mut client = connect(&name);
    write_message(
        &mut client,
        &ClientMessage::OpenSession {
            session: SESSION,
            app: None,
            protocol: PROTOCOL_VERSION,
        },
    )
    .unwrap();

    // 还没有上屏内容：提示帧，键已吃掉。
    write_message(
        &mut client,
        &ClientMessage::Key {
            session: SESSION,
            event: speak_combo(),
        },
    )
    .unwrap();
    let message: ServerMessage = read_message(&mut client)
        .expect("read response")
        .expect("server closed early");
    assert!(matches!(
        message,
        ServerMessage::KeyResult { outcome, frame, .. }
            if outcome == qingjian_platform::protocol::KeyOutcome::Consumed && frame.is_empty()
    ));
    write_message(&mut client, &ClientMessage::Poll { session: SESSION }).unwrap();
    let message: ServerMessage = read_message(&mut client)
        .expect("read response")
        .expect("server closed early");
    let ServerMessage::Update { frame, .. } = message else {
        panic!("应回 Update，实际 {message:?}");
    };
    assert_eq!(
        frame.candidates.items.first().map(|c| c.text.as_str()),
        Some("还没有上屏的内容")
    );

    // 打一句并上屏，再触发朗读：译文应交给 TTS 线程。
    for c in "nihao".chars() {
        write_message(
            &mut client,
            &ClientMessage::Key {
                session: SESSION,
                event: letter(c),
            },
        )
        .unwrap();
        let _: Option<ServerMessage> = read_message(&mut client).expect("read response");
    }
    write_message(
        &mut client,
        &ClientMessage::Key {
            session: SESSION,
            event: space(),
        },
    )
    .unwrap();
    let _: Option<ServerMessage> = read_message(&mut client).expect("read response");
    write_message(
        &mut client,
        &ClientMessage::Key {
            session: SESSION,
            event: speak_combo(),
        },
    )
    .unwrap();
    let _: Option<ServerMessage> = read_message(&mut client).expect("read response");
    write_message(&mut client, &ClientMessage::Poll { session: SESSION }).unwrap();
    let message: ServerMessage = read_message(&mut client)
        .expect("read response")
        .expect("server closed early");
    let ServerMessage::Update { frame, .. } = message else {
        panic!("应回 Update，实际 {message:?}");
    };
    assert_eq!(
        frame.candidates.items.first().map(|c| c.text.as_str()),
        Some("Hello world"),
        "朗读帧应显示译文"
    );
    let command = speech_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("应有 TTS 命令");
    assert!(matches!(
        command,
        SpeechCommand::Speak { ref text, language: Language::English }
            if text == "Hello world"
    ));
}
