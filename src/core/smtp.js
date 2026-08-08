/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Zero-dependency SMTP submission client (node:net + node:tls) — enough for
// invoice email delivery: greeting → EHLO → STARTTLS (587) or implicit TLS
// (465) → AUTH PLAIN → MAIL/RCPT/DATA/QUIT, with a multipart/mixed MIME
// builder for the invoice PDF attachment. Configuration comes from the
// environment only (credentials never live in the repo):
//   BUKIO_SMTP_HOST, BUKIO_SMTP_PORT (default 587), BUKIO_SMTP_SECURE=1
//   (implicit TLS on 465), BUKIO_SMTP_USER, BUKIO_SMTP_PASS, BUKIO_SMTP_FROM
import net from 'node:net';
import tls from 'node:tls';
import { randomBytes } from 'node:crypto';

export const SMTP_TIMEOUT_MS = 30_000;

export function smtpError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

export function smtpConfig() {
  const secure = process.env.BUKIO_SMTP_SECURE === '1';
  const portRaw = process.env.BUKIO_SMTP_PORT ?? (secure ? '465' : '587');
  return {
    host: process.env.BUKIO_SMTP_HOST ?? null,
    port: Number(portRaw),
    secure,
    user: process.env.BUKIO_SMTP_USER ?? null,
    pass: process.env.BUKIO_SMTP_PASS ?? null,
    from: process.env.BUKIO_SMTP_FROM ?? null,
  };
}

/** Validate the config (throws SMTP_NOT_CONFIGURED); also used by dry-run. */
export function smtpValidate(cfg) {
  const missing = [];
  if (!cfg.host) missing.push('BUKIO_SMTP_HOST');
  if (!cfg.from) missing.push('BUKIO_SMTP_FROM');
  if (!Number.isInteger(cfg.port) || cfg.port <= 0) missing.push(`BUKIO_SMTP_PORT ('${cfg.port}')`);
  if (missing.length > 0) {
    throw smtpError('SMTP_NOT_CONFIGURED', `SMTP is not configured — set ${missing.join(', ')}`);
  }
  return cfg;
}

/** UTF-8 base64 encoded-word for non-ASCII headers (Dutch accents). */
function encodeWord(value) {
  if (/^[\x20-\x7E]*$/.test(value) && value.length <= 60) return value;
  return `=?UTF-8?B?${Buffer.from(value, 'utf8').toString('base64')}?=`;
}

/** Base64 wrapped at 76 chars for the MIME body. */
function wrapBase64(b64) {
  const out = [];
  for (let i = 0; i < b64.length; i += 76) out.push(b64.slice(i, i + 76));
  return out.join('\r\n');
}

/**
 * Build a multipart/mixed MIME message: text part + optional attachment.
 * attachment: { filename, mime, dataBase64 }.
 */
export function buildMime({ from, to, subject, text, attachment = null }) {
  const boundary = `BUKIO-${randomBytes(12).toString('hex')}`;
  const lines = [
    `From: ${encodeWord(from)}`,
    `To: ${to}`,
    `Subject: ${encodeWord(subject)}`,
    'MIME-Version: 1.0',
    `Content-Type: multipart/mixed; boundary="${boundary}"`,
    '',
    `--${boundary}`,
    'Content-Type: text/plain; charset=utf-8',
    'Content-Transfer-Encoding: 8bit',
    '',
    text,
    '',
  ];
  if (attachment) {
    const name = encodeWord(attachment.filename);
    lines.push(
      `--${boundary}`,
      `Content-Type: ${attachment.mime}; name="${name}"`,
      `Content-Disposition: attachment; filename="${name}"`,
      'Content-Transfer-Encoding: base64',
      '',
      wrapBase64(attachment.dataBase64),
      '',
    );
  }
  lines.push(`--${boundary}--`, '');
  return lines.join('\r\n');
}

/** Line reader over a socket: complete \r\n lines, queueing early arrivals. */
function makeLineReader(socket) {
  let buf = '';
  const waiters = [];
  const pending = [];
  socket.on('data', (chunk) => {
    buf += chunk.toString('utf8');
    let idx;
    while ((idx = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, idx).replace(/\r$/, '');
      buf = buf.slice(idx + 1);
      const w = waiters.shift();
      if (w) w(line);
      else pending.push(line);
    }
  });
  return () => {
    if (pending.length) return Promise.resolve(pending.shift());
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        const i = waiters.indexOf(done);
        if (i >= 0) waiters.splice(i, 1);
        reject(smtpError('SMTP_CONNECT_FAILED', 'SMTP server did not respond (timeout)'));
      }, SMTP_TIMEOUT_MS);
      function done(line) {
        clearTimeout(timer);
        resolve(line);
      }
      waiters.push(done);
    });
  };
}

/**
 * Read SMTP replies until a final line (code + space) whose code is in
 * `accepted`; continuation lines (code + '-') are skipped. On a rejected
 * final line, onFail(rc, text) builds the error.
 */
async function expectReply(line, accepted, onFail) {
  for (;;) {
    const reply = await line();
    const m = reply.match(/^(\d{3})([ -])(.*)$/);
    if (!m) continue;
    if (m[2] === ' ') {
      const rc = Number(m[1]);
      if (accepted.includes(rc)) return reply;
      throw onFail(rc, m[3]);
    }
    // continuation — keep reading
  }
}

const failConnect = (rc, msg) => smtpError('SMTP_CONNECT_FAILED', `SMTP ${rc}: ${msg}`);
const failSend = (rc, msg) => smtpError('SMTP_SEND_FAILED', `SMTP ${rc}: ${msg}`);
const failAuth = (rc, msg) => smtpError('SMTP_AUTH_FAILED', `SMTP ${rc}: ${msg}`);

function writeLine(socket, cmd) {
  return new Promise((resolve) => socket.write(`${cmd}\r\n`, resolve));
}

/**
 * Read a multi-line reply (250-... continuations) until the final line;
 * returns ALL lines so the caller can inspect advertised extensions.
 */
async function ehloReply(line) {
  const lines = [];
  for (;;) {
    const reply = await line();
    const m = reply.match(/^(\d{3})([ -])(.*)$/);
    if (!m) continue;
    lines.push(reply);
    if (m[2] === ' ') {
      if (Number(m[1]) !== 250) throw failSend(Number(m[1]), m[3]);
      return lines;
    }
  }
}

function connectSocket(raw, host, port, secure) {
  return new Promise((resolve, reject) => {
    const fail = (err) => reject(smtpError('SMTP_CONNECT_FAILED', `cannot connect to ${host}:${port} — ${err.message}`));
    raw.once('error', fail);
    raw.once('connect', () => {
      raw.off('error', fail);
      if (!secure) return resolve(raw);
      const upgraded = tls.connect({ socket: raw, servername: host });
      upgraded.once('error', fail);
      upgraded.once('secureConnect', () => {
        upgraded.off('error', fail);
        resolve(upgraded);
      });
    });
  });
}

/**
 * Send one message. Returns { accepted: true, server } on 2xx.
 * Errors: SMTP_NOT_CONFIGURED / SMTP_CONNECT_FAILED / SMTP_AUTH_FAILED /
 * SMTP_SEND_FAILED (with the server's reply text).
 */
export async function sendMail({ host, port, secure, user, pass, from, to, subject, text, attachment = null }) {
  smtpValidate({ host, port, secure, user, pass, from });
  if (!to) throw smtpError('SMTP_SEND_FAILED', 'no recipient');

  let socket = null;
  let usedTls = secure;
  try {
    socket = await connectSocket(net.connect({ host, port }), host, port, secure);
    socket.setTimeout(SMTP_TIMEOUT_MS);
    socket.on('timeout', () => socket.destroy());
    let line = makeLineReader(socket);

    await expectReply(line, [220], failConnect); // greeting
    await writeLine(socket, 'EHLO bukio');
    const ehlo = await ehloReply(line); // multi-line; carries advertised extensions
    const hasStarttls = ehlo.some((l) => /^250[- ]STARTTLS/i.test(l));

    if (!secure && hasStarttls) {
      await writeLine(socket, 'STARTTLS');
      await expectReply(line, [220], failConnect);
      socket = await new Promise((resolve, reject) => {
        const upgraded = tls.connect({ socket, servername: host });
        const fail = (err) => reject(smtpError('SMTP_CONNECT_FAILED', `STARTTLS failed — ${err.message}`));
        upgraded.once('error', fail);
        upgraded.once('secureConnect', () => {
          upgraded.off('error', fail);
          resolve(upgraded);
        });
      });
      socket.setTimeout(SMTP_TIMEOUT_MS);
      socket.on('timeout', () => socket.destroy());
      line = makeLineReader(socket);
      usedTls = true;
      await writeLine(socket, 'EHLO bukio');
      await expectReply(line, [250], failSend);
    }

    if (user) {
      const token = Buffer.from(`\u0000${user}\u0000${pass}`, 'utf8').toString('base64');
      await writeLine(socket, `AUTH PLAIN ${token}`);
      let reply = await line();
      if (/^334/.test(reply)) {
        await writeLine(socket, token);
        reply = await line();
      }
      if (!/^235/.test(reply)) throw failAuth(reply.slice(0, 3), reply.slice(4));
    }

    await writeLine(socket, `MAIL FROM:<${from}>`);
    await expectReply(line, [250], failSend);
    await writeLine(socket, `RCPT TO:<${to}>`);
    await expectReply(line, [250, 251], failSend); // 251 = will forward
    await writeLine(socket, 'DATA');
    await expectReply(line, [354], failSend);
    await writeLine(socket, buildMime({ from, to, subject, text, attachment }));
    await writeLine(socket, '.');
    await expectReply(line, [250], failSend);
    await writeLine(socket, 'QUIT');
    try { await line(); } catch { /* QUIT reply is optional */ }

    return { accepted: true, server: `${host}:${port}`, tls: usedTls };
  } finally {
    if (socket) socket.destroy();
  }
}
