import { useEffect } from "react";

import { matches } from "../workbench/commands";
import type { Workbench } from "../workbench/workbench";

/**
 * Global keyboard shortcuts. Listens in the capture phase, so a shortcut works
 * wherever focus is, including the editor and the terminal. Only exact matches
 * are taken; every other key reaches the focused element untouched.
 */
export function useShortcuts(workbench: Workbench): void {
  useEffect(() => {
    const commands = workbench.commands();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.isComposing || workbench.dialogs.get() !== null) return;
      const command = commands.find((c) => c.shortcut && matches(c.shortcut, event));
      if (!command) return;
      event.preventDefault();
      event.stopPropagation();
      command.run();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [workbench]);
}
