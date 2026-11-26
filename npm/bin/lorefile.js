#!/usr/bin/env node
// The `lorefile` command for npm and npx.
//
// This package holds no binary. The first time it runs, this script downloads
// the Lorefile binary built for this computer from the project's GitHub
// Releases (the release whose version matches this package), checks its SHA-256
// checksum, and keeps it in a cache folder. From then on it just starts that
// binary with your arguments and exits with its exit code. Nothing runs at
// install time.
//
// Environment, all optional:
//   LOREFILE_BINARY        run this binary instead (say, a local build); nothing is downloaded
//   LOREFILE_REPO          the GitHub org/repo to download from (DEFAULT_REPO below)
//   LOREFILE_DOWNLOAD_URL  a folder URL holding the release assets: a mirror, or a test server
//   XDG_CACHE_HOME         where the cache lives: <it>/lorefile/<version>/ (default ~/.cache)
//
// Everything this script prints goes to stderr, so `npx lorefile mcp` keeps
// stdout for the MCP messages.
'use strict';

// The GitHub org/repo that publishes the binaries. Change it here, in
// npm/package.json, in scripts/install.sh and in .github/workflows/release.yml
// (LOREFILE_REPO) together; the release workflow checks they agree.
const DEFAULT_REPO = 'lorefile/lorefile';

const childProcess = require('child_process');
const crypto = require('crypto');
const fs = require('fs');
const os = require('os');
const path = require('path');

const VERSION = JSON.parse(fs.readFileSync(path.join(__dirname, '..', 'package.json'), 'utf8')).version;

// Node's platform and CPU names, mapped to the Rust targets the release workflow
// builds (keep in step with its build matrix).
const TARGETS = {
  'darwin arm64': 'aarch64-apple-darwin',
  'darwin x64': 'x86_64-apple-darwin',
  'linux arm64': 'aarch64-unknown-linux-gnu',
  'linux x64': 'x86_64-unknown-linux-gnu',
  'win32 x64': 'x86_64-pc-windows-msvc',
};
const PLATFORM_NAMES = { darwin: 'macOS', linux: 'Linux', win32: 'Windows' };

const REPO = process.env.LOREFILE_REPO || DEFAULT_REPO;
const OTHER_WAYS =
  `Other ways to run Lorefile: Docker (ghcr.io/${REPO.split('/')[0].toLowerCase()}/lorefile), ` +
  '`cargo install lorefile --locked`, or build it yourself and set LOREFILE_BINARY to the binary.';

let progressLineOpen = false; // a progress line without its newline yet

function say(line) {
  if (progressLineOpen) process.stderr.write('\n');
  progressLineOpen = false;
  process.stderr.write(`lorefile: ${line}\n`);
}

function fail(message) {
  say(message);
  process.exit(1);
}

// glibc builds do not run on musl Linux (Alpine). Node's report says which libc it runs on.
function isMusl() {
  if (process.platform !== 'linux') return false;
  try {
    const header = process.report.getReport().header;
    return !header.glibcVersionRuntime;
  } catch (_) {
    return false;
  }
}

function cacheDir() {
  let root = process.env.XDG_CACHE_HOME;
  if (!root && process.platform === 'win32' && process.env.LOCALAPPDATA) root = process.env.LOCALAPPDATA;
  if (!root) root = path.join(os.homedir(), '.cache');
  return path.join(root, 'lorefile', VERSION);
}

// GET a URL, following redirects (GitHub sends release downloads to a CDN).
// Resolves with the response stream; rejects with err.status set on HTTP errors.
function get(url, redirects = 0) {
  return new Promise((resolve, reject) => {
    const client = url.startsWith('http://') ? require('http') : require('https');
    const req = client.get(url, { headers: { 'User-Agent': `lorefile-npm/${VERSION}` } }, (res) => {
      const { statusCode, headers } = res;
      if ([301, 302, 303, 307, 308].includes(statusCode) && headers.location) {
        res.resume();
        const next = new URL(headers.location, url).toString();
        if (redirects >= 10) return reject(new Error(`too many redirects fetching ${url}`));
        if (url.startsWith('https:') && !next.startsWith('https:')) {
          return reject(new Error(`refusing a redirect from https to ${next}`));
        }
        return resolve(get(next, redirects + 1));
      }
      if (statusCode !== 200) {
        res.resume();
        const err = new Error(`HTTP ${statusCode} fetching ${url}`);
        err.status = statusCode;
        err.network = true;
        return reject(err);
      }
      resolve(res);
    });
    req.on('error', (err) => {
      err.network = true; // no connection, DNS, TLS, a proxy in the way, or the timeout below
      reject(err);
    });
    req.setTimeout(60000, () => req.destroy(new Error(`no answer for 60 seconds fetching ${url}`)));
  });
}

async function getText(url) {
  const res = await get(url);
  const chunks = [];
  for await (const chunk of res) chunks.push(chunk);
  return Buffer.concat(chunks).toString('utf8');
}

// `checksums.txt` lines look like "<64 hex digits>  <file name>".
function findChecksum(text, fileName) {
  for (const line of text.split(/\r?\n/)) {
    const m = line.trim().match(/^([0-9a-fA-F]{64})\s+\*?(.+)$/);
    if (m && m[2] === fileName) return m[1].toLowerCase();
  }
  return null;
}

// The published checksum for one asset. A package published by the release
// workflow carries the release's checksums.txt, so the download is checked
// against what npm holds (a mirror included); otherwise use the .sha256 file
// next to the asset. Resolves null when the asset is not published.
async function publishedChecksum(base, fileName) {
  const baked = path.join(__dirname, '..', 'checksums.txt');
  if (fs.existsSync(baked)) {
    return findChecksum(fs.readFileSync(baked, 'utf8'), fileName);
  }
  try {
    const text = await getText(`${base}/${fileName}.sha256`);
    const hex = text.trim().split(/\s+/)[0].toLowerCase();
    if (!/^[0-9a-f]{64}$/.test(hex)) throw new Error(`${fileName}.sha256 does not hold a SHA-256 checksum`);
    return hex;
  } catch (err) {
    if (err.status === 404) return null;
    throw err;
  }
}

// Stream a URL into a file, showing one progress line, and return the file's SHA-256.
async function download(url, file, label) {
  const res = await get(url);
  const total = Number(res.headers['content-length']) || 0;
  const mb = (n) => (n / 1048576).toFixed(1);
  const tty = process.stderr.isTTY;
  let received = 0;
  let shown = -1;
  const show = () => {
    const pct = total ? Math.floor((received / total) * 100) : 0;
    if (pct === shown) return;
    shown = pct;
    const amount = total ? `${pct}% of ${mb(total)} MB` : `${mb(received)} MB`;
    process.stderr.write(`\rlorefile: downloading ${label}… ${amount}`);
    progressLineOpen = true;
  };
  if (tty) show();
  else say(`downloading ${label}${total ? ` (${mb(total)} MB)` : ''}…`);

  const hash = crypto.createHash('sha256');
  const out = fs.createWriteStream(file, { mode: 0o600 });
  try {
    await new Promise((resolve, reject) => {
      res.on('data', (chunk) => {
        hash.update(chunk);
        received += chunk.length;
        if (tty) show();
      });
      res.on('aborted', () => reject(new Error(`the download of ${url} was cut off`)));
      res.on('error', reject);
      out.on('error', reject);
      out.on('finish', resolve);
      res.pipe(out);
    });
  } finally {
    out.destroy(); // let go of the file even when the download failed (Windows cannot delete open files)
  }
  if (progressLineOpen) process.stderr.write('\n');
  progressLineOpen = false;
  if (total && received !== total) throw new Error(`got ${received} of ${total} bytes from ${url}`);
  return hash.digest('hex');
}

// Unpack with the system's tar: on macOS and Linux it reads .tar.gz, and the
// bsdtar that ships with Windows 10 and later reads .zip.
function unpack(archive, into) {
  const win = process.platform === 'win32';
  const tar = win ? path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe') : 'tar';
  const args = win ? ['-xf', archive, '-C', into] : ['-xzf', archive, '-C', into];
  const r = childProcess.spawnSync(tar, args, { stdio: ['ignore', 'ignore', 'inherit'] });
  if (r.error || r.status !== 0) {
    throw new Error(`could not unpack ${path.basename(archive)} with ${tar}${r.error ? `: ${r.error.message}` : ''}`);
  }
}

// The cached binary for this version, downloading it first if needed.
async function cachedBinary() {
  const exe = process.platform === 'win32' ? 'lorefile.exe' : 'lorefile';
  const dir = cacheDir();
  const binary = path.join(dir, exe);
  if (fs.existsSync(binary)) return binary;

  const target = TARGETS[`${process.platform} ${process.arch}`];
  if (!target || isMusl()) {
    const what = isMusl() ? 'musl Linux (Alpine)' : `${process.platform} on ${process.arch}`;
    fail(`there is no prebuilt Lorefile binary for ${what}. ${OTHER_WAYS}`);
  }

  const base = (process.env.LOREFILE_DOWNLOAD_URL || `https://github.com/${REPO}/releases/download/v${VERSION}`).replace(/\/+$/, '');
  const ext = process.platform === 'win32' ? 'zip' : 'tar.gz';

  // The full build (with DuckDB) first. If a release has no full build for this
  // platform (Windows may ship slim only), use the slim one and say so.
  let name = null;
  let expected = null;
  for (const suffix of ['', '-slim']) {
    const candidate = `lorefile-${VERSION}-${target}${suffix}`;
    expected = await publishedChecksum(base, `${candidate}.${ext}`);
    if (expected) {
      name = candidate;
      break;
    }
  }
  if (!name) {
    fail(`Lorefile ${VERSION} has no build for ${target} at ${base}. Is ${VERSION} a published release? ${OTHER_WAYS}`);
  }
  if (name.endsWith('-slim')) {
    say(`this release has no full build for ${PLATFORM_NAMES[process.platform]}; using lorefile-slim (everything but SQL over local files).`);
  }

  fs.mkdirSync(dir, { recursive: true });
  const scratch = fs.mkdtempSync(path.join(dir, '.download-'));
  try {
    const archive = path.join(scratch, `${name}.${ext}`);
    const label = `Lorefile ${VERSION} for ${PLATFORM_NAMES[process.platform]} ${process.arch === 'x64' ? 'x86_64' : process.arch}`;
    const actual = await download(`${base}/${name}.${ext}`, archive, label);
    if (actual !== expected) {
      throw new Error(`checksum mismatch for ${name}.${ext}: expected ${expected}, got ${actual}. Nothing was installed.`);
    }
    unpack(archive, scratch);
    const unpacked = path.join(scratch, name, exe);
    if (!fs.existsSync(unpacked)) throw new Error(`${name}.${ext} does not contain ${name}/${exe}`);
    fs.chmodSync(unpacked, 0o755);
    // Rename is atomic, so a second `npx lorefile` started at the same moment
    // either finds the finished binary or does its own download.
    try {
      fs.renameSync(unpacked, binary);
    } catch (err) {
      if (!fs.existsSync(binary)) throw err;
    }
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true });
  }
  return binary;
}

// Start the binary with our arguments and our stdin, stdout and stderr; pass
// on signals; leave with its exit code (or its signal).
function run(binary, args) {
  const child = childProcess.spawn(binary, args, { stdio: 'inherit' });
  const signals = ['SIGINT', 'SIGTERM', 'SIGHUP'];
  for (const signal of signals) {
    process.on(signal, () => child.kill(signal));
  }
  child.on('error', (err) => fail(`could not start ${binary}: ${err.message}`));
  child.on('exit', (code, signal) => {
    if (signal) {
      for (const s of signals) process.removeAllListeners(s);
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code === null ? 1 : code);
  });
}

async function main() {
  let binary;
  if (process.env.LOREFILE_BINARY) {
    binary = path.resolve(process.env.LOREFILE_BINARY);
    if (!fs.existsSync(binary)) fail(`LOREFILE_BINARY points at ${binary}, which does not exist.`);
  } else {
    binary = await cachedBinary();
  }
  run(binary, process.argv.slice(2));
}

main().catch((err) => {
  // A network failure (no connection, a proxy, an HTTP error) gets the manual route as a hint.
  fail(
    err.network
      ? `${err.message}\nBehind a proxy or offline? Download the archive for this computer from ` +
          `https://github.com/${REPO}/releases/tag/v${VERSION} and set LOREFILE_BINARY to the binary inside. ${OTHER_WAYS}`
      : err.message
  );
});
