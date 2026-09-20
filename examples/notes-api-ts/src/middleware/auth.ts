import { config } from "../config";

export function checkAuth(req: { token?: string }): boolean {
  return req.token === config.token;
}
