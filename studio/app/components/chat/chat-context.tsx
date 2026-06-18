/**
 * Chat context for sharing callbacks across the message rendering tree.
 *
 * Avoids deep prop threading for CommandCard action handlers.
 *
 * @module components/chat/chat-context
 */

import { createContext, useContext } from "react";

interface ChatContextValue {
  /** Handle CommandCard action button clicks (e.g., "proceed_plan") */
  onCommandCardAction?: (actionId: string, metadata?: { planId?: string }) => void;
  /** Whether the chat is currently streaming */
  isStreaming: boolean;
}

export const ChatContext = createContext<ChatContextValue>({ isStreaming: false });

export const useChatContext = () => useContext(ChatContext);
