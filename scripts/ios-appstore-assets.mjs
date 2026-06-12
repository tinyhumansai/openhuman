#!/usr/bin/env node
import { createRequire } from "node:module";
import { mkdir, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(__dirname, "..");
const requireFromApp = createRequire(path.join(rootDir, "app/package.json"));
const { chromium } = requireFromApp("@playwright/test");

const outDir = path.join(rootDir, "fastlane/screenshots/en-US");
const width = 1320;
const height = 2868;

async function pngDataUri(relativePath) {
  const buffer = await readFile(path.join(rootDir, relativePath));
  return `data:image/png;base64,${buffer.toString("base64")}`;
}

function escapeHtml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function shell(content) {
  return `
    <div class="phone">
      <div class="status"><span>9:41</span><span>5G</span></div>
      ${content}
      <div class="home"></div>
    </div>
  `;
}

function pairScreen() {
  return shell(`
    <section class="pair">
      <div class="qr-mark">
        <div></div><div></div><div></div><span></span>
      </div>
      <h2>Pair with your desktop</h2>
      <p>Scan the QR code from OpenHuman on your computer and connect your phone in seconds.</p>
      <button>Scan QR code</button>
      <ol>
        <li>Open OpenHuman on desktop</li>
        <li>Go to Settings > Devices</li>
        <li>Tap Pair phone to show QR</li>
      </ol>
    </section>
  `);
}

function chatScreen() {
  return shell(`
    <section class="chat">
      <header>
        <small>Connected to</small>
        <strong>Desktop</strong>
      </header>
      <div class="avatar">
        <div class="face">
          <span></span><span></span><i></i>
        </div>
      </div>
      <div class="messages">
        <p class="assistant">I found the latest thread context from your desktop workspace.</p>
        <p class="user">Summarize what needs my attention.</p>
        <p class="assistant">You have two follow-ups, one meeting note, and a draft ready to send.</p>
      </div>
      <footer>
        <div class="mic"></div>
        <div class="input">Type a message...</div>
        <div class="send"></div>
      </footer>
    </section>
  `);
}

function privacyScreen() {
  return shell(`
    <section class="privacy">
      <div class="lock">
        <span></span>
      </div>
      <h2>Anchored to your desktop</h2>
      <p>Your phone is a companion surface. Memory, tools, and integrations stay with the OpenHuman runtime you paired.</p>
      <div class="rows">
        <div><strong>Short-lived QR pairing</strong><span>Connect intentionally</span></div>
        <div><strong>Push-to-talk voice</strong><span>Speak when you choose</span></div>
        <div><strong>Desktop-owned context</strong><span>Use the assistant you already set up</span></div>
      </div>
    </section>
  `);
}

function html({ title, kicker, body, iconUri, wordmarkUri }) {
  return `<!doctype html>
  <html>
    <head>
      <meta charset="utf-8" />
      <style>
        * { box-sizing: border-box; }
        html, body { margin: 0; width: ${width}px; height: ${height}px; overflow: hidden; }
        body {
          font-family: Inter, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
          background: #10131a;
          color: white;
        }
        .shot {
          position: relative;
          width: ${width}px;
          height: ${height}px;
          padding: 122px 92px 108px;
          background:
            radial-gradient(circle at 20% 14%, rgba(74, 131, 221, 0.42), transparent 34%),
            linear-gradient(160deg, #121720 0%, #111827 48%, #18231f 100%);
        }
        .brand { display: flex; align-items: center; gap: 22px; height: 82px; }
        .brand img.icon { width: 82px; height: 82px; border-radius: 22px; }
        .brand img.wordmark { width: 330px; height: auto; }
        .copy { margin-top: 104px; width: 100%; }
        .kicker {
          color: #9abff9;
          font-size: 34px;
          font-weight: 700;
          letter-spacing: 0;
          margin-bottom: 26px;
        }
        h1 {
          font-family: "Cabinet Grotesk", Inter, -apple-system, BlinkMacSystemFont, sans-serif;
          font-size: 86px;
          line-height: 0.96;
          letter-spacing: 0;
          margin: 0;
          max-width: 1000px;
        }
        .subtitle {
          margin-top: 34px;
          color: rgba(255, 255, 255, 0.76);
          font-size: 38px;
          line-height: 1.3;
          max-width: 930px;
        }
        .phone {
          position: absolute;
          left: 50%;
          bottom: 94px;
          transform: translateX(-50%);
          width: 780px;
          height: 1510px;
          border-radius: 88px;
          background: #0f1117;
          border: 18px solid #202735;
          box-shadow: 0 52px 120px rgba(0, 0, 0, 0.48);
          overflow: hidden;
        }
        .status {
          height: 78px;
          padding: 24px 54px 0;
          display: flex;
          justify-content: space-between;
          font-size: 24px;
          color: rgba(255, 255, 255, 0.76);
        }
        .home {
          position: absolute;
          left: 50%;
          bottom: 26px;
          transform: translateX(-50%);
          width: 210px;
          height: 10px;
          border-radius: 999px;
          background: rgba(255, 255, 255, 0.42);
        }
        .pair, .privacy {
          min-height: calc(100% - 78px);
          padding: 230px 58px 110px;
          display: flex;
          flex-direction: column;
          align-items: center;
          text-align: center;
        }
        .qr-mark {
          width: 154px;
          height: 154px;
          border-radius: 34px;
          background: #4a83dd;
          display: grid;
          grid-template-columns: repeat(2, 1fr);
          gap: 16px;
          padding: 28px;
          box-shadow: 0 22px 54px rgba(74, 131, 221, 0.36);
        }
        .qr-mark div, .qr-mark span {
          background: white;
          border-radius: 12px;
          opacity: 0.92;
        }
        .pair h2, .privacy h2 {
          font-size: 48px;
          line-height: 1.06;
          margin: 54px 0 20px;
          letter-spacing: 0;
        }
        .pair p, .privacy p {
          margin: 0;
          font-size: 27px;
          line-height: 1.42;
          color: rgba(255, 255, 255, 0.62);
        }
        .pair button {
          margin-top: 70px;
          width: 100%;
          height: 96px;
          border: 0;
          border-radius: 26px;
          background: #4a83dd;
          color: white;
          font-size: 30px;
          font-weight: 700;
        }
        .pair ol {
          margin: 78px 0 0;
          padding: 0;
          list-style: none;
          width: 100%;
          text-align: left;
          display: grid;
          gap: 22px;
        }
        .pair li {
          padding: 26px 28px;
          border-radius: 24px;
          background: rgba(255, 255, 255, 0.08);
          color: rgba(255, 255, 255, 0.78);
          font-size: 25px;
        }
        .chat header {
          height: 112px;
          padding: 16px 34px;
          border-bottom: 1px solid rgba(255, 255, 255, 0.1);
          display: flex;
          flex-direction: column;
          justify-content: center;
        }
        .chat small {
          color: rgba(255, 255, 255, 0.42);
          font-size: 20px;
          text-transform: uppercase;
        }
        .chat strong { margin-top: 8px; font-size: 28px; }
        .avatar {
          height: 500px;
          display: flex;
          align-items: center;
          justify-content: center;
        }
        .face {
          width: 330px;
          height: 330px;
          border-radius: 50%;
          background: linear-gradient(145deg, #8bb4f4, #4a83dd 58%, #6cbf9a);
          position: relative;
          box-shadow: 0 26px 76px rgba(74, 131, 221, 0.42);
        }
        .face span {
          position: absolute;
          top: 120px;
          width: 38px;
          height: 48px;
          border-radius: 50%;
          background: #10131a;
        }
        .face span:first-child { left: 98px; }
        .face span:nth-child(2) { right: 98px; }
        .face i {
          position: absolute;
          left: 50%;
          bottom: 86px;
          transform: translateX(-50%);
          width: 108px;
          height: 34px;
          border-radius: 0 0 80px 80px;
          border-bottom: 16px solid #10131a;
        }
        .messages {
          padding: 0 34px;
          display: grid;
          gap: 20px;
        }
        .messages p {
          margin: 0;
          padding: 24px 26px;
          border-radius: 28px;
          font-size: 25px;
          line-height: 1.32;
        }
        .messages .assistant {
          background: rgba(255, 255, 255, 0.09);
          color: rgba(255, 255, 255, 0.82);
          justify-self: start;
          max-width: 570px;
        }
        .messages .user {
          background: #4a83dd;
          justify-self: end;
          max-width: 520px;
        }
        .chat footer {
          position: absolute;
          left: 0;
          right: 0;
          bottom: 54px;
          height: 116px;
          border-top: 1px solid rgba(255, 255, 255, 0.1);
          padding: 22px 30px;
          display: flex;
          align-items: center;
          gap: 18px;
        }
        .mic, .send {
          width: 70px;
          height: 70px;
          border-radius: 22px;
          background: #4a83dd;
        }
        .mic::before {
          content: "";
          display: block;
          width: 22px;
          height: 34px;
          margin: 16px auto 0;
          border-radius: 999px;
          border: 5px solid white;
        }
        .send::before {
          content: "";
          display: block;
          width: 24px;
          height: 24px;
          margin: 23px auto 0;
          border-top: 6px solid white;
          border-right: 6px solid white;
          transform: rotate(45deg);
        }
        .input {
          flex: 1;
          height: 70px;
          border-radius: 22px;
          background: rgba(255, 255, 255, 0.1);
          color: rgba(255, 255, 255, 0.36);
          font-size: 24px;
          display: flex;
          align-items: center;
          padding: 0 24px;
        }
        .lock {
          width: 164px;
          height: 164px;
          border-radius: 40px;
          background: #6cbf9a;
          position: relative;
          box-shadow: 0 22px 54px rgba(108, 191, 154, 0.3);
        }
        .lock::before {
          content: "";
          position: absolute;
          left: 46px;
          top: 36px;
          width: 72px;
          height: 62px;
          border: 14px solid white;
          border-bottom: 0;
          border-radius: 42px 42px 0 0;
        }
        .lock span {
          position: absolute;
          left: 38px;
          bottom: 34px;
          width: 88px;
          height: 70px;
          border-radius: 16px;
          background: white;
        }
        .rows {
          margin-top: 78px;
          width: 100%;
          display: grid;
          gap: 20px;
        }
        .rows div {
          text-align: left;
          padding: 26px 28px;
          border-radius: 24px;
          background: rgba(255, 255, 255, 0.08);
        }
        .rows strong {
          display: block;
          font-size: 27px;
          margin-bottom: 8px;
        }
        .rows span {
          color: rgba(255, 255, 255, 0.56);
          font-size: 23px;
        }
      </style>
    </head>
    <body>
      <main class="shot">
        <div class="brand">
          <img class="icon" src="${iconUri}" alt="">
          <img class="wordmark" src="${wordmarkUri}" alt="">
        </div>
        <section class="copy">
          <div class="kicker">${escapeHtml(kicker)}</div>
          <h1>${escapeHtml(title)}</h1>
          <p class="subtitle">${escapeHtml(body)}</p>
        </section>
        ${body.includes("QR") ? pairScreen() : title.includes("voice") ? chatScreen() : privacyScreen()}
      </main>
    </body>
  </html>`;
}

const shots = [
  {
    file: "iPhone_6_9_01_pair_with_desktop.png",
    kicker: "OpenHuman for iPhone",
    title: "Pair with your desktop",
    body: "Scan the QR code from OpenHuman on your computer and connect your phone to the assistant you already trust.",
  },
  {
    file: "iPhone_6_9_02_chat_and_voice.png",
    kicker: "Text and push-to-talk",
    title: "Chat by text or voice",
    body: "Send quick messages, dictate thoughts, and get spoken replies while your desktop runtime does the work.",
  },
  {
    file: "iPhone_6_9_03_desktop_anchored.png",
    kicker: "Desktop-owned context",
    title: "Your memory stays anchored",
    body: "Use your paired OpenHuman setup for memory, tools, and integrations without turning the phone into a separate workspace.",
  },
];

await mkdir(outDir, { recursive: true });
const iconUri = await pngDataUri(
  "app/src-tauri-mobile/icons/store/appstore.png",
);
const wordmarkUri = await pngDataUri(
  "app/public/brand/OpenhumanLogo+wordmark-White.png",
);

const browser = await chromium.launch();
try {
  const page = await browser.newPage({
    viewport: { width, height },
    deviceScaleFactor: 1,
  });
  for (const shot of shots) {
    await page.setContent(html({ ...shot, iconUri, wordmarkUri }), {
      waitUntil: "load",
    });
    const destination = path.join(outDir, shot.file);
    await page.screenshot({ path: destination, fullPage: false });
    console.log(
      `[ios-appstore-assets] wrote ${path.relative(rootDir, destination)}`,
    );
  }
} finally {
  await browser.close();
}
