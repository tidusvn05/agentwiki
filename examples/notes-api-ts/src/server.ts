import { router } from "./routes/index";
import { checkAuth } from "./middleware/auth";
import { db } from "./infra/db";

export function startServer(port: number): void {
  db.connect();
  router.use(checkAuth);
  router.listen(port);
}
