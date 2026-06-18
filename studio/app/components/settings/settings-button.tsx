/**
 * Settings button component.
 *
 * Compact icon button that triggers the settings dialog.
 *
 * @module components/settings/settings-button
 */

import { SettingsIcon } from "lucide-react";
import { type ButtonHTMLAttributes, forwardRef } from "react";
import { cn } from "~/lib/utils";

export interface SettingsButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  "aria-label"?: string;
}

export const SettingsButton = forwardRef<HTMLButtonElement, SettingsButtonProps>(
  ({ onClick, className, disabled, "aria-label": ariaLabel, ...props }, ref) => {
    return (
      <button
        ref={ref}
        type="button"
        onClick={onClick}
        className={cn(
          "inline-flex items-center justify-center rounded-md p-2 text-muted-foreground",
          "hover:bg-muted hover:text-foreground transition-colors",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
          "disabled:pointer-events-none disabled:opacity-50",
          className
        )}
        disabled={disabled}
        aria-label={ariaLabel || "Settings"}
        title="Settings"
        {...props}
      >
        <SettingsIcon className="size-4" />
        <span className="sr-only">Settings</span>
      </button>
    );
  }
);

SettingsButton.displayName = "SettingsButton";
