/**
 * Server-sent events, split on blank lines.
 *
 * A trailing buffer that does not end in a newline is dropped: the sender
 * closed mid-line. A final event that ends on a newline but has no blank
 * line after it is still delivered.
 *
 * Deliberately not a Log entry: frames are wire bytes. The assistant entry
 * is the collected message, not these events.
 */

export type SseEvent = { event: string; data: string };

export async function* parseSse(
  body: ReadableStream<Uint8Array>,
): AsyncGenerator<SseEvent> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const pulled = pullEvents(buffer);
      buffer = pulled.rest;
      for (const event of pulled.events) yield event;
    }
    buffer += decoder.decode();
    const tail = flushTail(buffer);
    if (tail) yield tail;
  } finally {
    await reader.cancel().catch(() => undefined);
  }
}

function pullEvents(buffer: string): { events: SseEvent[]; rest: string } {
  const events: SseEvent[] = [];
  let rest = buffer;
  while (true) {
    const sep = findSeparator(rest);
    if (!sep) break;
    const block = rest.slice(0, sep.index);
    rest = rest.slice(sep.index + sep.length);
    const event = parseBlock(block);
    if (event) events.push(event);
  }
  return { events, rest };
}

function flushTail(buffer: string): SseEvent | undefined {
  if (buffer.length === 0 || !buffer.endsWith("\n")) return undefined;
  return parseBlock(buffer);
}

function findSeparator(
  buffer: string,
): { index: number; length: number } | undefined {
  const lf = buffer.indexOf("\n\n");
  const crlf = buffer.indexOf("\r\n\r\n");
  if (lf < 0 && crlf < 0) return undefined;
  if (lf < 0) return { index: crlf, length: 4 };
  if (crlf < 0) return { index: lf, length: 2 };
  return lf <= crlf ? { index: lf, length: 2 } : { index: crlf, length: 4 };
}

function parseBlock(block: string): SseEvent | undefined {
  let event = "";
  const data: string[] = [];
  let sawData = false;
  for (const line of block.split(/\r?\n/)) {
    if (line.length === 0 || line.startsWith(":")) continue;
    const colon = line.indexOf(":");
    const field = colon === -1 ? line : line.slice(0, colon);
    let value = colon === -1 ? "" : line.slice(colon + 1);
    if (value.startsWith(" ")) value = value.slice(1);
    if (field === "event") event = value;
    else if (field === "data") {
      sawData = true;
      data.push(value);
    }
  }
  if (!sawData) return undefined;
  return { event, data: data.join("\n") };
}
