// Copies the site into dist/, the way a bundler would.
import { cpSync, mkdirSync, rmSync } from "node:fs";

rmSync("dist", { recursive: true, force: true });
mkdirSync("dist");
for (const entry of ["index.html", "style.css", "app.js", "img", "fonts"]) {
  cpSync(entry, "dist/" + entry, { recursive: true });
}
