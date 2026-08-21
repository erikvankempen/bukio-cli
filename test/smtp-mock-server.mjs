/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Standalone SMTP mock for tests that spawn the CLI via execFileSync — an
// in-process mock can't service connections while spawnSync blocks the
// parent's event loop, so this fixture runs as its own process. Prints
// `PORT=<n>` on stdout once listening; serves until killed.
// Flags: --fail-auth --fail-rcpt --fail-greeting --advertise-starttls
//        --reject-starttls --auth-user U --auth-pass P
import net from 'node:net';

const argv = process.argv.slice(2);
const flag = (name) => argv.includes(`--${name}`);
const opt = (name, dflt) => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : dflt;
};

const auth = { user: opt('auth-user', 'u'), pass: opt('auth-pass', 'p') };
const failAuth = flag('fail-auth');
const failRcpt = flag('fail-rcpt');
const failGreeting = flag('fail-greeting');
const advertiseStarttls = flag('advertise-starttls');
const rejectStarttls = flag('reject-starttls');

const server = net.createServer((sock) => {
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
          chunks.length = 0;
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

server.listen(0, '127.0.0.1', () => {
  console.log(`PORT=${server.address().port}`);
});
