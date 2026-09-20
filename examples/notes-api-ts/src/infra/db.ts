import { config } from "../config";

export const db = {
  connect(): void {
    void config.dbUrl;
  },
  async query<T>(_sql: string): Promise<T[]> {
    return [] as T[];
  },
};
