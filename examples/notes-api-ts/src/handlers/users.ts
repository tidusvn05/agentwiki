import { db } from "../infra/db";
import { User } from "../models";

export const usersRouter = {
  register(): void {},
  async list(): Promise<User[]> {
    return db.query<User>("SELECT * FROM users");
  },
};
