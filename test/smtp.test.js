/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { openDb } from '../src/core/db.js';
import { createContact, createInvoice, finalizeInvoice } from '../src/invoice/index.js';
import { emailInvoice } from '../src/invoice/email.js';
import { sendMail, buildMime, smtpConfig, smtpValidate } from '../src/core/smtp.js';

const BIN = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'bukio.js');

function cli(dbPath, args, { expectFail = false, env = {} } = {}) {
  const fullEnv = { ...process.env, BUKIO_DB: dbPath, BUKIO_ACTOR: 'agent:test', ...env };
  try {
    const stdout = execFileSync(process.execPath, [BIN, '--json', ...args], { env: fullEnv, encoding: 'utf8' });
    return { code: 0, out: JSON.parse(stdout) };
  } catch (err) {
    if (expectFail) return { code: err.status, out: JSON.parse(err.stdout), err: err.stderr };
    throw err;
  }
}

/**
 * Minimal in-process SMTP server for tests. Tracks connections, captures the
 * DATA payload, and can be told to reject auth / RCPT / greeting.
 */
function smtpMock({ auth = { user: 'u', pass: 'p' }, failAuth = false, failRcpt = false, failGreeting = false, advertiseStarttls = false, rejectStarttls = false } = {}) {
  const state = { connections: 0, lastData: null, port: null, close: null };
  const server = net.createServer((sock) => {
    state.connections += 1;
    let buf = '';
    let inData = false;
    const chunks = [];
    const send = (line) => sock.write(`${line}\r\n`);
    send(failGreeting ? '554 no service' : '220 mock ESMTP');
    sock.on('data', (chunk) => {
      buf += chunk.toString('utf8');
      let idx;
      while ((idx = buf.indexOf('\r\n')) >= 0) {
        const cmd = buf.slice(0, idx);
        buf = buf.slice(idx + 2);
        if (inData) {
          if (cmd === '.') {
            inData = false;
            state.lastData = chunks.join('\r\n');
            send('250 2.0.0 ok: queued');
            continue;
          }
          chunks.push(cmd);
          continue;
        }
        const up = cmd.toUpperCase();
        if (up.startsWith('EHLO')) {
          send('250-mock');
          if (advertiseStarttls) send('250-STARTTLS');
          send('250 AUTH PLAIN');
        } else if (up.startsWith('AUTH PLAIN')) {
          const token = cmd.slice(11).trim();
          const expected = Buffer.from(`\u0000${auth.user}\u0000${auth.pass}`, 'utf8').toString('base64');
          if (!failAuth && token === expected) send('235 2.7.0 ok');
          else send('535 5.7.8 authentication failed');
        } else if (up.startsWith('MAIL FROM')) {
          send('250 2.1.0 ok');
        } else if (up.startsWith('RCPT TO')) {
          send(failRcpt ? '550 5.1.1 no such user' : '250 2.1.5 ok');
        } else if (up.startsWith('DATA')) {
          inData = true;
          chunks.length = 0;
          send('354 go');
        } else if (up.startsWith('STARTTLS')) {
          send(rejectStarttls ? '454 4.7.0 TLS not available' : '220 go ahead');
        } else if (up.startsWith('QUIT')) {
          send('221 bye');
          sock.end();
        } else {
          send('250 ok');
        }
      }
    });
    sock.on('error', () => {});
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      state.port = server.address().port;
      state.close = () => server.close();
      resolve(state);
    });
  });
}

let t;
let db;
let file;

test.beforeEach(() => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-smtp-test-'));
  file = path.join(dir, 'test.db');
  cli(file, ['init', '--name', 'Test Coaching', '--kvk', '12345678', '--legal-form', 'eenmanszaak', '--vat', 'off']);
  cli(file, ['company', 'update', '--address', 'Teststraat 1', '--postal-code', '1000 AA', '--city', 'Amsterdam']);
  db = openDb(file);
  t = dir;
});
test.afterEach(() => {
  db.close();
  rmSync(t, { recursive: true, force: true });
});

function seedInvoice({ email = 'klant@acme.example' } = {}) {
  const contact = createContact(db, {
    name: 'Acme BV', address: 'Klantstraat 1', city: 'Amsterdam', email, actor: 'agent:test',
  });
  const inv = createInvoice(db, { contactId: contact.id, lines: ['Ding @ 100.00'], date: '2026-08-10', actor: 'agent:test' });
  return finalizeInvoice(db, { id: inv.id, actor: 'agent:test' }).invoice;
}

test('sendMail: happy path delivers, captures the MIME with the PDF attachment', async () => {
  const mock = await smtpMock();
  try {
    const r = await sendMail({
      host: '127.0.0.1', port: mock.port, secure: false, user: 'u', pass: 'p',
      from: 'no-reply@test.example', to: 'x@y.example',
      subject: 'Factuur 2026-0001 — Test Coaching',
      text: 'Geachte,',
      attachment: { filename: '2026-0001.pdf', mime: 'application/pdf', dataBase64: Buffer.from('%PDF-1.4 fake').toString('base64') },
    });
    assert.equal(r.accepted, true);
    assert.equal(r.tls, false); // mock does not advertise STARTTLS → plaintext
    // the subject has a non-ASCII em-dash, so it ships as a UTF-8 encoded-word
    assert.match(mock.lastData, /^Subject: =\?UTF-8\?B\?/m);
    assert.match(mock.lastData, /Content-Type: multipart\/mixed/);
    assert.match(mock.lastData, /application\/pdf/);
    assert.match(mock.lastData, /2026-0001\.pdf/);
    // base64 attachment decodes to the PDF bytes
    const b64 = mock.lastData.split('\r\n').filter((l) => !l.includes(':') && l.length > 0 && !l.startsWith('--') && !l.startsWith('Geachte')).join('');
    const decoded = Buffer.from(b64, 'base64').toString('utf8');
    assert.equal(decoded, '%PDF-1.4 fake');
  } finally {
    mock.close();
  }
});

test('sendMail: auth failure → SMTP_AUTH_FAILED', async () => {
  const mock = await smtpMock({ failAuth: true });
  try {
    await assert.rejects(
      sendMail({ host: '127.0.0.1', port: mock.port, secure: false, user: 'u', pass: 'wrong', from: 'a@b.c', to: 'x@y.z', subject: 's', text: 't' }),
      (e) => e.code === 'SMTP_AUTH_FAILED',
    );
  } finally {
    mock.close();
  }
});

test('sendMail: rcpt rejection → SMTP_SEND_FAILED with server text', async () => {
  const mock = await smtpMock({ failRcpt: true });
  try {
    await assert.rejects(
      sendMail({ host: '127.0.0.1', port: mock.port, secure: false, user: 'u', pass: 'p', from: 'a@b.c', to: 'x@y.z', subject: 's', text: 't' }),
      (e) => e.code === 'SMTP_SEND_FAILED' && /550/.test(e.message),
    );
  } finally {
    mock.close();
  }
});

test('sendMail: STARTTLS advertised but rejected → SMTP_CONNECT_FAILED (branch exercised)', async () => {
  const mock = await smtpMock({ advertiseStarttls: true, rejectStarttls: true });
  try {
    await assert.rejects(
      sendMail({ host: '127.0.0.1', port: mock.port, secure: false, user: 'u', pass: 'p', from: 'a@b.c', to: 'x@y.z', subject: 's', text: 't' }),
      (e) => e.code === 'SMTP_CONNECT_FAILED' && /454/.test(e.message),
    );
  } finally {
    mock.close();
  }
});

test('sendMail: connection refused → SMTP_CONNECT_FAILED; bad greeting → SMTP_CONNECT_FAILED', async () => {
  // grab a port that is closed
  const probe = net.createServer();
  await new Promise((resolve) => probe.listen(0, '127.0.0.1', resolve));
  const deadPort = probe.address().port;
  await new Promise((resolve) => probe.close(resolve));

  await assert.rejects(
    sendMail({ host: '127.0.0.1', port: deadPort, secure: false, from: 'a@b.c', to: 'x@y.z', subject: 's', text: 't' }),
    (e) => e.code === 'SMTP_CONNECT_FAILED',
  );

  const mock = await smtpMock({ failGreeting: true });
  try {
    await assert.rejects(
      sendMail({ host: '127.0.0.1', port: mock.port, secure: false, from: 'a@b.c', to: 'x@y.z', subject: 's', text: 't' }),
      (e) => e.code === 'SMTP_CONNECT_FAILED',
    );
  } finally {
    mock.close();
  }
});

test('smtpConfig/smtpValidate: env-driven; missing host/from → SMTP_NOT_CONFIGURED', () => {
  const prev = { ...process.env };
  try {
    delete process.env.BUKIO_SMTP_HOST;
    delete process.env.BUKIO_SMTP_FROM;
    assert.throws(() => smtpValidate(smtpConfig()), (e) => e.code === 'SMTP_NOT_CONFIGURED');
    process.env.BUKIO_SMTP_HOST = 'smtp.example';
    process.env.BUKIO_SMTP_FROM = 'a@b.c';
    process.env.BUKIO_SMTP_PORT = '587';
    const cfg = smtpConfig();
    assert.equal(cfg.port, 587);
    assert.equal(cfg.secure, false);
    process.env.BUKIO_SMTP_SECURE = '1';
    assert.equal(smtpConfig().secure, true);
    // an explicit PORT wins over the secure default (587 → 465)
    assert.equal(smtpConfig().port, 587);
    delete process.env.BUKIO_SMTP_PORT;
    assert.equal(smtpConfig().port, 465);
  } finally {
    process.env = prev;
  }
});

test('buildMime: non-ASCII subject → UTF-8 encoded-word; attachment boundary present', () => {
  const mime = buildMime({
    from: 'a@b.c', to: 'x@y.z', subject: 'Factuur met ééndag', text: 'hallo',
    attachment: { filename: 'a.pdf', mime: 'application/pdf', dataBase64: Buffer.from('abc').toString('base64') },
  });
  assert.match(mime, /^Subject: =\?UTF-8\?B\?/m);
  assert.match(mime, /Content-Disposition: attachment/);
  assert.match(mime, /--BUKIO-/);
  // plain ASCII subject stays readable
  const plain = buildMime({ from: 'a@b.c', to: 'x@y.z', subject: 'Factuur 2026-0001', text: 'hallo' });
  assert.match(plain, /^Subject: Factuur 2026-0001/m);
});

test('buildMime: CR/LF in to/subject/filename cannot inject headers', () => {
  const mime = buildMime({
    from: 'a@b.c\r\nBcc: victim@evil.example', to: 'x@y.z\r\nX-Evil: 1',
    subject: 'Factuur\r\nBcc: victim@evil.example', text: 'hallo',
    attachment: { filename: 'a.pdf\r\nX-Evil: 1', mime: 'application/pdf', dataBase64: 'YQ==' },
  });
  // no header may contain a raw line break
  for (const line of mime.split('\r\n')) {
    assert.ok(!/[\r\n]/.test(line), `header line must be single-line: ${JSON.stringify(line)}`);
  }
  // the injected text stays INSIDE the header value (stripped, merged) — it
  // never becomes its own header line
  assert.ok(mime.includes('a@b.cBcc: victim@evil.example'));
  const headerLines = mime.split('\r\n').filter((l) => !l.startsWith('--') && l.includes(':'));
  assert.ok(!headerLines.some((l) => l.startsWith('Bcc:')), 'no standalone Bcc: header may appear');
  assert.ok(!headerLines.some((l) => l.startsWith('X-Evil:')), 'no standalone X-Evil: header may appear');
});

test('sendMail: dot-stuffed payload — a body line starting with "." survives', async () => {
  const mock = await smtpMock();
  try {
    await sendMail({
      host: '127.0.0.1', port: mock.port, secure: false, user: 'u', pass: 'p',
      from: 'a@b.c', to: 'x@y.z', subject: 'T', text: 'Hallo\n.een aparte regel\nEinde',
    });
    assert.ok(mock.lastData.includes('..een aparte regel'), 'dot-stuffed line must be doubled in the DATA payload');
  } finally {
    mock.close();
  }
});

test('emailInvoice: delivers to the contact email and audits', async () => {
  const invoice = seedInvoice();
  const mock = await smtpMock();
  try {
    const prev = { ...process.env };
    process.env.BUKIO_SMTP_HOST = '127.0.0.1';
    process.env.BUKIO_SMTP_PORT = String(mock.port);
    process.env.BUKIO_SMTP_USER = 'u';
    process.env.BUKIO_SMTP_PASS = 'p';
    process.env.BUKIO_SMTP_FROM = 'no-reply@test.example';
    try {
      const r = await emailInvoice(db, { id: invoice.id, attachPdf: false, actor: 'agent:test' });
      assert.equal(r.delivered, true);
      assert.equal(r.to, 'klant@acme.example');
      assert.equal(r.invoice_number, invoice.invoice_number);
      const audit = db.prepare("SELECT * FROM audit_log WHERE action = 'invoice.email'").all();
      assert.equal(audit.length, 1);
      assert.equal(audit[0].actor, 'agent:test');
      assert.equal(JSON.parse(audit[0].args_json).to, 'klant@acme.example');
    } finally {
      process.env = prev;
    }
  } finally {
    mock.close();
  }
});

test('emailInvoice: guards — draft, missing email, unconfigured SMTP', async () => {
  const contact = createContact(db, { name: 'Acme BV', address: 'Klantstraat 1', city: 'Amsterdam', actor: 'agent:test' });
  const draft = createInvoice(db, { contactId: contact.id, lines: ['Ding @ 100.00'], date: '2026-08-10', actor: 'agent:test' });
  await assert.rejects(emailInvoice(db, { id: draft.id, attachPdf: false }), (e) => e.code === 'NOT_FINALIZED');

  const inv = finalizeInvoice(db, { id: draft.id, actor: 'agent:test' }).invoice;
  // no email on contact, no --to
  await assert.rejects(emailInvoice(db, { id: inv.id, attachPdf: false }), (e) => e.code === 'CONTACT_EMAIL_MISSING');
  // --to override works, but SMTP unconfigured
  await assert.rejects(
    emailInvoice(db, { id: inv.id, to: 'x@y.z', attachPdf: false, dryRun: false }),
    (e) => e.code === 'SMTP_NOT_CONFIGURED',
  );
});

test('emailInvoice: dry-run renders the plan, makes no connection, audits nothing', async () => {
  const invoice = seedInvoice();
  const mock = await smtpMock();
  try {
    const prev = { ...process.env };
    process.env.BUKIO_SMTP_HOST = '127.0.0.1';
    process.env.BUKIO_SMTP_PORT = String(mock.port);
    process.env.BUKIO_SMTP_FROM = 'no-reply@test.example';
    try {
      const plan = await emailInvoice(db, { id: invoice.id, attachPdf: false, actor: 'agent:test', dryRun: true });
      assert.equal(plan.dryRun, true);
      assert.equal(plan.to, 'klant@acme.example');
      assert.equal(mock.connections, 0, 'dry-run must not open a connection');
      assert.equal(db.prepare("SELECT COUNT(*) c FROM audit_log WHERE action = 'invoice.email'").get().c, 0);
    } finally {
      process.env = prev;
    }
  } finally {
    mock.close();
  }
});

test('emailInvoice: PDF attachment is rendered and decodes to %PDF', async () => {
  const invoice = seedInvoice();
  const mock = await smtpMock();
  try {
    const prev = { ...process.env };
    process.env.BUKIO_SMTP_HOST = '127.0.0.1';
    process.env.BUKIO_SMTP_PORT = String(mock.port);
    process.env.BUKIO_SMTP_USER = 'u';
    process.env.BUKIO_SMTP_PASS = 'p';
    process.env.BUKIO_SMTP_FROM = 'no-reply@test.example';
    try {
      const r = await emailInvoice(db, { id: invoice.id, actor: 'agent:test' });
      assert.equal(r.delivered, true);
      assert.match(mock.lastData, /application\/pdf/);
      assert.match(mock.lastData, /2026-\d{4}\.pdf/);
      // the base64 payload decodes to a PDF
      const pdfB64 = mock.lastData.split('\r\n')
        .filter((l) => !l.includes(':') && !l.includes('--') && !l.startsWith('Geachte') && l.length > 0)
        .join('');
      const decoded = Buffer.from(pdfB64, 'base64').toString('utf8');
      assert.match(decoded, /^%PDF/);
    } finally {
      process.env = prev;
    }
  } finally {
    mock.close();
  }
});

test('cli: invoice email e2e with SMTP env + audit row', async () => {
  const invoice = seedInvoice();
  // the mock must run in its OWN process: execFileSync blocks this test's
  // event loop, so an in-process server could never answer the CLI child
  const mockProc = spawn(process.execPath, [path.join(import.meta.dirname, 'smtp-mock-server.mjs')], { stdio: ['ignore', 'pipe', 'ignore'] });
  let mockPort = null;
  let mockBuf = '';
  await new Promise((resolve) => {
    mockProc.stdout.on('data', (c) => {
      mockBuf += c.toString();
      const m = mockBuf.match(/PORT=(\d+)/);
      if (m) { mockPort = Number(m[1]); resolve(); }
    });
  });
  try {
    const env = {
      BUKIO_SMTP_HOST: '127.0.0.1', BUKIO_SMTP_PORT: String(mockPort),
      BUKIO_SMTP_USER: 'u', BUKIO_SMTP_PASS: 'p', BUKIO_SMTP_FROM: 'no-reply@test.example',
    };
    const dry = cli(file, ['invoice', 'email', '--id', String(invoice.id), '--no-pdf', '--dry-run'], { env });
    assert.equal(dry.out.data.dryRun, true);

    const sent = cli(file, ['invoice', 'email', '--id', String(invoice.id), '--no-pdf'], { env });
    assert.equal(sent.code, 0);
    assert.equal(sent.out.data.delivered, true);

    const d = openDb(file);
    try {
      const row = d.prepare("SELECT * FROM audit_log WHERE action = 'invoice.email' ORDER BY id DESC LIMIT 1").get();
      assert.ok(row);
      assert.equal(row.actor, 'agent:test');
    } finally {
      d.close();
    }

    const unconfigured = cli(file, ['invoice', 'email', '--id', String(invoice.id), '--no-pdf'], { expectFail: true, env: {} });
    assert.equal(unconfigured.out.error.code, 'SMTP_NOT_CONFIGURED');
  } finally {
    mockProc.kill();
  }
});

test('mcp: invoice_email dry-run parity (no connection) + execute', async () => {
  const invoice = seedInvoice();
  const mock = await smtpMock();
  const mcp = spawn(process.execPath, [BIN, 'mcp', '--db', file], {
    cwd: path.join(path.dirname(fileURLToPath(import.meta.url)), '..'),
    env: {
      ...process.env, BUKIO_ACTOR: 'agent:test',
      BUKIO_SMTP_HOST: '127.0.0.1', BUKIO_SMTP_PORT: String(mock.port),
      BUKIO_SMTP_USER: 'u', BUKIO_SMTP_PASS: 'p', BUKIO_SMTP_FROM: 'no-reply@test.example',
    },
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  let buf = '';
  let nextId = 1;
  const pending = new Map();
  mcp.stdout.on('data', (chunk) => {
    buf += chunk;
    let idx;
    while ((idx = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, idx);
      buf = buf.slice(idx + 1);
      if (!line.trim()) continue;
      let msg;
      try { msg = JSON.parse(line); } catch { continue; }
      if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
    }
  });
  const call = (method, params) => {
    const id = nextId++;
    return new Promise((resolve) => {
      pending.set(id, resolve);
      mcp.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
    });
  };
  try {
    await call('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 't', version: '1' } });
    const plan = await call('tools/call', { name: 'invoice_email', arguments: { id: invoice.id, attach_pdf: false } });
    const planData = JSON.parse(plan.result.content[0].text);
    assert.equal(planData.mode, 'dry-run');
    assert.equal(mock.connections, 0);

    const exec = await call('tools/call', { name: 'invoice_email', arguments: { id: invoice.id, attach_pdf: false, mode: 'execute' } });
    const execData = JSON.parse(exec.result.content[0].text);
    assert.equal(execData.mode, 'execute');
    assert.equal(execData.delivered, true);
    assert.equal(mock.connections, 1);
  } finally {
    mcp.kill();
    mock.close();
  }
});
