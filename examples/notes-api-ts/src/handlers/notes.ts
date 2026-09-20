import { db } from "../infra/db";
import { Note } from "../models";

export const notesRouter = {
  register(): void {},
  async list(): Promise<Note[]> {
    return db.query<Note>("SELECT * FROM notes");
  },
};
