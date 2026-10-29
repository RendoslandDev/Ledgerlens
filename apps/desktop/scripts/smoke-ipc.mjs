// End-to-end smoke test for a release build on Windows, run in CI.
//
// Launches a build whose window opens WebView2's remote debugging port (see
// .github/scripts/smoke-windows.ps1), then drives the real IPC from inside the page: checks
// system_info, imports a text PDF as raw bytes (the same call the Add button makes), and waits for
// the background reader to extract it.
// On failure it prints the app's and WebView2's processes, listening ports, and data folder.
//
// Usage: node scripts/smoke-ipc.mjs <app.exe> <fixture.pdf>
import { execFileSync, spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { basename } from "node:path";

const [exe, fixture] = process.argv.slice(2);
if (!exe || !fixture) {
  console.error("usage: node scripts/smoke-ipc.mjs <app.exe> <fixture.pdf>");
  process.exit(2);
}
const PORT = 9333;
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const app = spawn(exe, [], { stdio: "inherit" });
let exited = null;
app.on("exit", (code) => (exited = code));

async function main() {
  let page;
  let lastSeen = "nothing yet";
  for (let attempt = 0; attempt < 60 && !page; attempt++) {
    if (exited !== null) throw new Error(`the app exited early with code ${exited}`);
    await wait(1000);
    try {
      const targets = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      lastSeen = `targets ${JSON.stringify(targets.map(({ type, url }) => ({ type, url })))}`;
      page = targets.find((target) => target.type === "page");
    } catch (error) {
      lastSeen = `debug port: ${error.cause?.code ?? error.message}`;
    }
    if (attempt % 10 === 9) console.log(`waiting for the page (${attempt + 1} s): ${lastSeen}`);
  }
  if (!page) throw new Error(`no WebView2 page appeared within 60 s; last: ${lastSeen}`);
  console.log(`page: ${page.url}`);

  const socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });
  let nextId = 0;
  const pending = new Map();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    pending.get(message.id)?.(message);
    pending.delete(message.id);
  });
  // Evaluates an async expression in the page and returns its JSON value.
  const evaluate = (expression) =>
    new Promise((resolve, reject) => {
      const id = ++nextId;
      pending.set(id, (message) => {
        const result = message.result;
        if (message.error) reject(new Error(message.error.message));
        else if (result?.exceptionDetails) reject(new Error(JSON.stringify(result.exceptionDetails)));
        else resolve(result?.result?.value);
      });
      socket.send(
        JSON.stringify({
          id,
          method: "Runtime.evaluate",
          params: { expression, awaitPromise: true, returnByValue: true },
        }),
      );
    });

  for (let attempt = 0; ; attempt++) {
    if (await evaluate("typeof window.__TAURI_INTERNALS__?.invoke === 'function'")) break;
    if (attempt > 30) throw new Error("the Tauri IPC never became available");
    await wait(500);
  }
  const invoke = (command, args, options) =>
    evaluate(
      `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${args}, ${JSON.stringify(options ?? {})})` +
        `.then(value => ({ ok: true, value }), error => ({ ok: false, error }))`,
    );

  const info = await invoke("system_info", "{}");
  if (!info?.ok) throw new Error(`system_info failed: ${JSON.stringify(info)}`);
  console.log(`platform=${info.value.platform} ocr=${info.value.ocrAvailable} watching=${info.value.folderWatching}`);
  if (info.value.ocrAvailable || info.value.folderWatching) {
    throw new Error("OCR and folder watching should report unavailable off macOS");
  }

  const name = basename(fixture);
  const base64 = readFileSync(fixture).toString("base64");
  const imported = await invoke(
    "import_document",
    `Uint8Array.from(atob(${JSON.stringify(base64)}), (c) => c.charCodeAt(0))`,
    { headers: { "x-file-name": encodeURIComponent(name) } },
  );
  if (!imported?.ok) throw new Error(`import failed: ${JSON.stringify(imported)}`);
  console.log(`imported ${name}: ${imported.value.document.status}`);

  for (let attempt = 0; attempt < 60; attempt++) {
    const listed = await invoke("list_documents", "{}");
    const document = listed?.value?.find((entry) => entry.fileName === name);
    if (document && !["queued", "processing"].includes(document.status)) {
      console.log(`read: status=${document.status} source=${document.textSource} vendor=${document.vendor}`);
      if (document.textSource !== "text_layer" || !document.vendor?.includes("Kumasi")) {
        throw new Error(`unexpected extraction: ${JSON.stringify(document)}`);
      }
      return;
    }
    await wait(1000);
  }
  throw new Error("the import was not read within 60 s");
}

// What was running when the test failed: whether WebView2 started, with which arguments.
function diagnose() {
  const script = [
    "Get-CimInstance Win32_Process | Where-Object Name -match 'ledgerlens|msedgewebview2' |",
    "  ForEach-Object { '{0} {1}: {2}' -f $_.ProcessId, $_.Name, $_.CommandLine }",
    "Get-NetTCPConnection -State Listen | Where-Object LocalPort -ge 9000 |",
    "  ForEach-Object { 'listening {0}:{1} pid {2}' -f $_.LocalAddress, $_.LocalPort, $_.OwningProcess }",
    "Get-ChildItem (Join-Path $env:LOCALAPPDATA 'com.ledgerlens.app') -Depth 1 -ErrorAction SilentlyContinue |",
    "  ForEach-Object { $_.FullName }",
  ].join("\n");
  try {
    console.log(execFileSync("powershell", ["-NoProfile", "-Command", script], { encoding: "utf8" }));
  } catch (error) {
    console.log(`diagnostics failed: ${error.message}`);
  }
}

main()
  .then(() => {
    console.log("smoke test passed");
    app.kill();
    process.exit(0);
  })
  .catch((error) => {
    console.error(`smoke test failed: ${error.message}`);
    if (process.platform === "win32") diagnose();
    app.kill();
    process.exit(1);
  });
