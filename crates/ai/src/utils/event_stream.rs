// SSE bytes → AssistantMessageEvent stream — Rust port of pi-mono utils/event-stream.ts.
//
// pi-mono's EventStream is a push-based queue; here we pull from a bytes `Stream`
// and parse SSE lines on the fly, which fits Rust's poll model better and avoids
// any internal queue allocation.

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::Stream;

use crate::stream::parse_sse_chunk_multi;
use crate::types::AssistantMessageEvent;

/// A `Stream` adapter that decodes SSE bytes into `AssistantMessageEvent`s.
///
/// Corresponds to pi-mono's `EventStream` / `AssistantMessageEventStream`.
/// Internally maintains a line buffer (for multi-chunk SSE lines) and a
/// small pending-event queue (for frames that yield multiple events at once,
/// e.g. DashScope tool-call name + arguments in one frame).
pub struct EventStream<S> {
    /// Upstream bytes source.
    inner: S,
    /// Accumulates bytes until a newline is found.
    line_buf: Vec<u8>,
    /// Events already parsed but not yet yielded.
    pending: std::collections::VecDeque<AssistantMessageEvent>,
    /// Set when the upstream is exhausted.
    done: bool,
}

impl<S> EventStream<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            line_buf: Vec::new(),
            pending: std::collections::VecDeque::new(),
            done: false,
        }
    }
}

/// Wrap a byte `Stream` as an `EventStream`.
///
/// The byte stream is typically `response.bytes_stream()` from reqwest.
pub fn event_stream<S, E>(bytes: S) -> EventStream<S>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
{
    EventStream::new(bytes)
}

// ---------------------------------------------------------------------------
// SSE line → events
// ---------------------------------------------------------------------------

/// Process one complete SSE text line into the pending queue.
fn process_line(line: &str, pending: &mut std::collections::VecDeque<AssistantMessageEvent>) {
    let line = line.trim_end_matches('\r'); // handle CRLF
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') {
        return;
    }
    if let Some(data) = line.strip_prefix("data: ") {
        match parse_sse_chunk_multi(data) {
            Ok(evts) => pending.extend(evts),
            Err(e) => tracing::warn!("EventStream SSE parse error: {e}, data: {data}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Stream impl
// ---------------------------------------------------------------------------

impl<S, E> Stream for EventStream<S>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    type Item = AssistantMessageEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            // Drain any events we've already parsed.
            if let Some(event) = this.pending.pop_front() {
                return Poll::Ready(Some(event));
            }

            if this.done {
                return Poll::Ready(None);
            }

            // Pull more bytes from upstream.
            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Pending => return Poll::Pending,

                Poll::Ready(None) => {
                    // Upstream exhausted. Flush any remaining bytes in the buffer
                    // (final chunk may lack a trailing newline).
                    this.done = true;
                    if !this.line_buf.is_empty() {
                        let tail = String::from_utf8_lossy(&this.line_buf).into_owned();
                        this.line_buf.clear();
                        for line in tail.lines() {
                            process_line(line, &mut this.pending);
                        }
                    }
                    // Loop once more to drain pending (or return None).
                }

                Poll::Ready(Some(Err(e))) => {
                    this.done = true;
                    let msg = AssistantMessageEvent::Error(format!("Stream read error: {e}"));
                    return Poll::Ready(Some(msg));
                }

                Poll::Ready(Some(Ok(chunk))) => {
                    this.line_buf.extend_from_slice(&chunk);

                    // Extract all complete lines from the buffer.
                    while let Some(pos) = this.line_buf.iter().position(|&b| b == b'\n') {
                        let line_bytes = this.line_buf[..pos].to_vec();
                        this.line_buf.drain(..=pos);
                        let line = String::from_utf8_lossy(&line_bytes);
                        process_line(&line, &mut this.pending);
                    }
                    // Loop to drain pending before polling upstream again.
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StopReason;
    use futures::StreamExt;

    /// Build an `EventStream` from a slice of raw SSE byte chunks.
    fn make_stream(
        chunks: Vec<&'static str>,
    ) -> EventStream<impl Stream<Item = Result<Bytes, std::io::Error>> + Unpin> {
        let byte_stream = futures::stream::iter(
            chunks
                .into_iter()
                .map(|s| Ok::<Bytes, std::io::Error>(Bytes::from(s))),
        );
        event_stream(byte_stream)
    }

    #[tokio::test]
    async fn test_single_text_delta() {
        let mut s = make_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"},\"index\":0}]}\n",
        ]);
        let ev = s.next().await.unwrap();
        assert!(matches!(ev, AssistantMessageEvent::TextDelta(t) if t == "hello"));
    }

    #[tokio::test]
    async fn test_multiple_chunks() {
        let mut s = make_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"index\":0}]}\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" there\"},\"index\":0}]}\n",
            "data: [DONE]\n",
        ]);
        let mut text = String::new();
        while let Some(ev) = s.next().await {
            if let AssistantMessageEvent::TextDelta(t) = ev {
                text.push_str(&t);
            }
        }
        assert_eq!(text, "hi there");
    }

    #[tokio::test]
    async fn test_line_split_across_chunks() {
        // The SSE line is split across two byte chunks.
        let part1 = "data: {\"choices\":[{\"delta\":{\"content\":\"spl";
        let part2 = "it\"},\"index\":0}]}\n";
        let mut s = make_stream(vec![part1, part2]);
        let ev = s.next().await.unwrap();
        assert!(matches!(ev, AssistantMessageEvent::TextDelta(t) if t == "split"));
    }

    #[tokio::test]
    async fn test_done_event() {
        let mut s = make_stream(vec![
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\",\"index\":0}]}\n",
        ]);
        let ev = s.next().await.unwrap();
        assert!(
            matches!(ev, AssistantMessageEvent::Done { stop_reason } if stop_reason == StopReason::Stop)
        );
    }

    #[tokio::test]
    async fn test_stream_error_propagated() {
        let byte_stream = futures::stream::iter(vec![
            Ok::<Bytes, std::io::Error>(Bytes::from(
                "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"index\":0}]}\n",
            )),
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "pipe broken",
            )),
        ]);
        let mut s = event_stream(byte_stream);
        let ev1 = s.next().await.unwrap();
        assert!(matches!(ev1, AssistantMessageEvent::TextDelta(_)));
        let ev2 = s.next().await.unwrap();
        assert!(matches!(ev2, AssistantMessageEvent::Error(msg) if msg.contains("pipe broken")));
    }

    #[tokio::test]
    async fn test_comments_and_empty_lines_skipped() {
        let mut s = make_stream(vec![
            ": keep-alive\n",
            "\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"A\"},\"index\":0}]}\n",
        ]);
        let ev = s.next().await.unwrap();
        assert!(matches!(ev, AssistantMessageEvent::TextDelta(t) if t == "A"));
        assert!(s.next().await.is_none());
    }

    #[tokio::test]
    async fn test_no_trailing_newline_flushed() {
        // Final chunk without newline should still be parsed.
        let mut s = make_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"end\"},\"index\":0}]}",
        ]);
        let ev = s.next().await.unwrap();
        assert!(matches!(ev, AssistantMessageEvent::TextDelta(t) if t == "end"));
    }

    #[tokio::test]
    async fn test_dashscope_multi_event_per_frame() {
        // DashScope sends tool name + arguments in one SSE frame →
        // parse_sse_chunk_multi should yield ToolCallStart + ToolCallDelta.
        let data = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,",
            "\"id\":\"call_x\",\"type\":\"function\",",
            "\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"path\\\":\\\"/tmp\\\"}\"}}]},",
            "\"index\":0}]}\n"
        );
        let mut s = make_stream(vec![data]);
        let ev1 = s.next().await.unwrap();
        assert!(matches!(ev1, AssistantMessageEvent::ToolCallStart { .. }));
        let ev2 = s.next().await.unwrap();
        assert!(matches!(ev2, AssistantMessageEvent::ToolCallDelta { .. }));
    }
}
