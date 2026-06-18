/**
 * Error boundary component for catching and displaying errors.
 *
 * Features:
 * - Catches React render errors
 * - Development vs production error display
 * - Reset capability for recovery
 * - Preserves user context where possible
 *
 * @module components/error-boundary
 */

import { Component, type ErrorInfo, type ReactNode } from "react";

// ============================================================================
// Types
// ============================================================================

interface ErrorBoundaryProps {
  /** Child components to render */
  children: ReactNode;
  /** Optional fallback UI to render on error */
  fallback?: ReactNode;
  /** Callback when error occurs */
  onError?: (error: Error, errorInfo: ErrorInfo) => void;
  /** Callback when reset is triggered */
  onReset?: () => void;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
  errorInfo: ErrorInfo | null;
}

// ============================================================================
// Error Fallback UI (Development-only detailed errors)
// ============================================================================

interface ErrorFallbackProps {
  error: Error | null;
  errorInfo: ErrorInfo | null;
  onReset: () => void;
}

function ErrorFallback({ error, errorInfo, onReset }: ErrorFallbackProps) {
  const isDev = import.meta.env?.DEV ?? false;

  return (
    <div className="flex items-center justify-center min-h-[200px] p-6">
      <div className="max-w-lg w-full">
        {/* Error Header */}
        <div className="flex items-center gap-3 mb-4">
          <div className="w-10 h-10 rounded-full bg-destructive/10 flex items-center justify-center flex-shrink-0">
            <svg
              className="w-6 h-6 text-destructive"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              aria-hidden="true"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
              />
            </svg>
          </div>
          <div>
            <h2 className="text-lg font-semibold text-foreground">Something went wrong</h2>
            <p className="text-sm text-muted-foreground">
              {isDev ? "See details below" : "Please try again or refresh the page"}
            </p>
          </div>
        </div>

        {/* Error Details (Development only) */}
        {isDev && error && (
          <div className="mb-4 p-4 bg-[var(--accent-red)] border border-destructive/20 rounded-lg">
            <p className="font-mono text-sm text-[var(--dot-red)] break-all">{error.message}</p>
            {errorInfo?.componentStack && (
              <details className="mt-3">
                <summary className="text-xs text-destructive cursor-pointer hover:underline">
                  Component stack trace
                </summary>
                <pre className="mt-2 text-xs text-destructive overflow-auto max-h-48 whitespace-pre-wrap">
                  {errorInfo.componentStack}
                </pre>
              </details>
            )}
          </div>
        )}

        {/* Actions */}
        <div className="flex gap-3">
          <button
            type="button"
            onClick={onReset}
            className="flex-1 px-4 py-2 bg-primary text-primary-foreground text-sm font-medium rounded-lg hover:bg-primary/90 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none transition-colors"
          >
            Try again
          </button>
          <button
            type="button"
            onClick={() => window.location.reload()}
            className="px-4 py-2 bg-muted text-muted-foreground text-sm font-medium rounded-lg hover:bg-muted/80 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none transition-colors"
          >
            Refresh page
          </button>
        </div>
      </div>
    </div>
  );
}

// ============================================================================
// Error Boundary Component
// ============================================================================

/**
 * Error boundary component that catches JavaScript errors in child components.
 *
 * @example
 * ```tsx
 * <ErrorBoundary onError={logError}>
 *   <ChatArea threadId={threadId} />
 * </ErrorBoundary>
 * ```
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = {
      hasError: false,
      error: null,
      errorInfo: null,
    };
  }

  static getDerivedStateFromError(error: Error): Partial<ErrorBoundaryState> {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo): void {
    this.setState({ errorInfo });

    // Call optional error callback
    this.props.onError?.(error, errorInfo);

    // Development-only console logging
    if (import.meta.env?.DEV) {
      console.error("[ErrorBoundary] Caught error:", error);
      console.error("[ErrorBoundary] Component stack:", errorInfo.componentStack);
    }
  }

  handleReset = (): void => {
    this.setState({
      hasError: false,
      error: null,
      errorInfo: null,
    });
    this.props.onReset?.();
  };

  render(): ReactNode {
    if (this.state.hasError) {
      // Use custom fallback if provided
      if (this.props.fallback) {
        return this.props.fallback;
      }

      // Default fallback UI
      return (
        <ErrorFallback
          error={this.state.error}
          errorInfo={this.state.errorInfo}
          onReset={this.handleReset}
        />
      );
    }

    return this.props.children;
  }
}

// ============================================================================
// Specialized Error Boundaries
// ============================================================================

interface ChatErrorBoundaryProps {
  children: ReactNode;
  threadId?: string;
}

/**
 * Specialized error boundary for chat components.
 * Provides thread-aware error handling and recovery.
 */
export function ChatErrorBoundary({ children, threadId }: ChatErrorBoundaryProps) {
  const handleError = (error: Error, _errorInfo: ErrorInfo) => {
    // Log in development only
    if (import.meta.env?.DEV) {
      console.error(`[ChatErrorBoundary] Error in thread ${threadId}:`, error);
    }

    // In production, you might send to error tracking service
    // e.g., Sentry.captureException(error, { extra: { threadId, componentStack: errorInfo.componentStack } });
  };

  return (
    <ErrorBoundary
      key={threadId ?? "chat"}
      onError={handleError}
      fallback={
        <div className="flex flex-col items-center justify-center h-full p-8 text-center">
          <div className="w-16 h-16 rounded-full bg-destructive/10 flex items-center justify-center mb-4">
            <svg
              className="w-8 h-8 text-destructive"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              aria-hidden="true"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
              />
            </svg>
          </div>
          <h3 className="text-lg font-medium text-foreground mb-2">Chat Error</h3>
          <p className="text-muted-foreground mb-4">Unable to display this conversation.</p>
          <button
            type="button"
            onClick={() => window.location.reload()}
            className="px-4 py-2 bg-primary text-primary-foreground text-sm font-medium rounded-lg hover:bg-primary/90 transition-colors"
          >
            Reload Chat
          </button>
        </div>
      }
    >
      {children}
    </ErrorBoundary>
  );
}

// ============================================================================
// Exports
// ============================================================================

export default ErrorBoundary;
