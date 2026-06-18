/**
 * Thread domain types for conversation management.
 *
 * @module domain/thread
 */

// ============================================================================
// Core Types
// ============================================================================

/**
 * Unique thread identifier.
 */
export type ThreadId = string;

/**
 * Resource/tenant identifier.
 */
export type ResourceId = string;

/**
 * Thread metadata.
 */
export interface Thread {
  readonly id: ThreadId;
  readonly title: string | null;
  readonly resourceId: ResourceId;
  readonly createdAt: string;
  readonly updatedAt: string;
}

/**
 * Thread creation request.
 */
export interface CreateThreadRequest {
  readonly title?: string;
}

/**
 * Thread update request.
 */
export interface UpdateThreadRequest {
  readonly title?: string;
}

// ============================================================================
// Constructors
// ============================================================================

/**
 * Create a new thread with defaults.
 */
export const createThread = (resourceId: ResourceId, title?: string): Thread => ({
  id: crypto.randomUUID(),
  title: title ?? "New Conversation",
  resourceId,
  createdAt: new Date().toISOString(),
  updatedAt: new Date().toISOString(),
});

/**
 * Generate a thread ID (UUID v4).
 */
export const generateThreadId = (): ThreadId => crypto.randomUUID();

// ============================================================================
// Thread List Operations
// ============================================================================

/**
 * Sort threads by updated time (most recent first).
 */
export const sortThreadsByRecent = (threads: Thread[]): Thread[] =>
  [...threads].sort(
    (a, b) => new Date(b.updatedAt || 0).getTime() - new Date(a.updatedAt || 0).getTime()
  );

/**
 * Find a thread by ID.
 */
export const findThread = (threads: Thread[], id: ThreadId): Thread | undefined =>
  threads.find((t) => t.id === id);

/**
 * Update a thread's timestamp (for "touch" operation).
 */
export const touchThread = (thread: Thread): Thread => ({
  ...thread,
  updatedAt: new Date().toISOString(),
});

/**
 * Update thread in a list.
 */
export const updateThreadInList = (
  threads: Thread[],
  id: ThreadId,
  update: Partial<Thread>
): Thread[] => threads.map((t) => (t.id === id ? { ...t, ...update } : t));

/**
 * Remove a thread from a list.
 */
export const removeThreadFromList = (threads: Thread[], id: ThreadId): Thread[] =>
  threads.filter((t) => t.id !== id);

// ============================================================================
// Title Generation
// ============================================================================

/**
 * Generate a title from the first message content.
 */
export const generateTitle = (content: string, maxLength = 50): string => {
  const cleaned = content.replace(/\s+/g, " ").trim().slice(0, maxLength);

  if (content.length > maxLength) {
    return `${cleaned}...`;
  }
  return cleaned || "New Conversation";
};

// ============================================================================
// Eval Threads
// ============================================================================

/**
 * Check if a thread ID belongs to an eval run.
 */
export const isEvalThread = (threadId: ThreadId | undefined): boolean =>
  typeof threadId === "string" && threadId.startsWith("eval-");

// ============================================================================
// Thread Files
// ============================================================================

/**
 * File associated with a thread.
 */
export interface ThreadFile {
  readonly fileId: string;
  readonly filename: string;
  readonly threadId: ThreadId;
  readonly createdAt: string;
}

/**
 * Sort files by creation time (most recent first).
 */
export const sortFilesByRecent = (files: ThreadFile[]): ThreadFile[] =>
  [...files].sort(
    (a, b) => new Date(b.createdAt || 0).getTime() - new Date(a.createdAt || 0).getTime()
  );
