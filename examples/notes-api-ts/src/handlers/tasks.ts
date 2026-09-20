import { db } from "../infra/db";
import { Task } from "../models";

export const tasksRouter = {
  register(): void {},
  async list(): Promise<Task[]> {
    return db.query<Task>("SELECT * FROM tasks");
  },
};
