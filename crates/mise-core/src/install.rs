pub use crate::runner::{ProcessStreamer, StreamEvent, StreamingRunner};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CoreError;
    use crate::runner::CommandRequest;
    use std::sync::mpsc::{Receiver, channel};

    struct FakeStreamer {
        events: Vec<StreamEvent>,
    }

    impl StreamingRunner for FakeStreamer {
        fn spawn(&self, _req: CommandRequest) -> Result<Receiver<StreamEvent>, CoreError> {
            let (tx, rx) = channel();
            for e in &self.events {
                tx.send(e.clone()).unwrap();
            }
            Ok(rx)
        }
    }

    #[test]
    fn forwards_events_and_exit_code() {
        let streamer = FakeStreamer {
            events: vec![
                StreamEvent::Stdout("downloading".into()),
                StreamEvent::Stderr("warn".into()),
                StreamEvent::Exited(0),
            ],
        };
        let rx = streamer
            .spawn(CommandRequest {
                program: "mise".into(),
                args: vec![],
                cwd: None,
            })
            .unwrap();
        let got: Vec<_> = rx.into_iter().collect();
        assert_eq!(got.len(), 3);
        assert!(matches!(got[2], StreamEvent::Exited(0)));
    }
}
