// Render the UI mockups (HTML in this folder) to docs/mockups/*.png.
// They show proposed UI that is not implemented. Needs Node and Playwright
// with Chromium; run from anywhere: node tools/mockups/render.js
// (if Playwright is installed globally: NODE_PATH=<global node_modules>).
// Pass file names (node tools/mockups/render.js 5-start.html) to render
// only those.
const { chromium } = require("playwright");
const fs = require("fs");
const path = require("path");

const here = __dirname;
const out = path.join(here, "..", "..", "docs", "mockups");
const pages = process.argv.length > 2
  ? process.argv.slice(2).map((f) => path.basename(f))
  : fs.readdirSync(here).filter((f) => /^\d-.*\.html$/.test(f)).sort();

(async () => {
  fs.mkdirSync(out, { recursive: true });
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1360, height: 860 }, deviceScaleFactor: 2 });
  for (const f of pages) {
    await page.goto("file://" + path.join(here, f));
    await page.waitForTimeout(200);
    await page.screenshot({ path: path.join(out, f.replace(/\.html$/, ".png")) });
    console.log("rendered", f);
  }
  await browser.close();
})();
