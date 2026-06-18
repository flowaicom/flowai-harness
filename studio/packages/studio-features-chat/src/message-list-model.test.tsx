import { describe, expect, test } from "bun:test";
import { createRef } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { getMessageText, getStreamingMessageId, serializeChatHistory } from "./message-list-model";
import { SharedVirtualizedMessageList } from "./virtualized-message-list";

describe("chat message list model", () => {
  test("serializes chat history from text parts only", () => {
    const messages = [
      {
        id: "message-1",
        role: "user",
        parts: [
          { type: "text", text: "Show " },
          { type: "tool-call", text: "ignored" },
          { type: "text", text: "orders" },
        ],
      },
      {
        id: "message-2",
        role: "assistant",
        parts: [{ type: "text", text: "Here are the results." }],
      },
    ] as const;

    expect(getMessageText(messages[0].parts)).toBe("Show orders");
    expect(serializeChatHistory(messages)).toEqual([
      { role: "user", content: "Show orders" },
      { role: "assistant", content: "Here are the results." },
    ]);
  });

  test("selects the final message as streaming only while stream is active", () => {
    const messages = [{ id: "message-1" }, { id: "message-2" }];

    expect(getStreamingMessageId(messages, false)).toBeUndefined();
    expect(getStreamingMessageId(messages, true)).toBe("message-2");
    expect(getStreamingMessageId([], true)).toBeUndefined();
  });

  test("renders the configured empty state without app dependencies", () => {
    const scrollContainerRef = createRef<HTMLDivElement>();
    type TestMessage = { readonly id: string };

    const html = renderToStaticMarkup(
      <SharedVirtualizedMessageList<TestMessage>
        messages={[]}
        scrollContainerRef={scrollContainerRef}
        emptyState={<div>No messages yet</div>}
        renderMessage={(message) => <div>{message.id}</div>}
      />
    );

    expect(html).toContain("No messages yet");
  });
});
