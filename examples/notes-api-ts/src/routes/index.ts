import { tasksRouter } from "../handlers/tasks";
import { notesRouter } from "../handlers/notes";

export const router = {
  use(_mw: unknown): void {},
  listen(_port: number): void {},
  mount(): void {
    tasksRouter.register();
    notesRouter.register();
  },
};
