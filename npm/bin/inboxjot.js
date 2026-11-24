#!/usr/bin/env node
const { spawn } = require('node:child_process');

const platform = `${process.platform}-${process.arch}`;
const extension = process.platform === 'win32' ? '.exe' : '';

let binary;
try {
  binary = require.resolve(`@inboxjot/cli-${platform}/bin/inboxjot${extension}`);
} catch {
  console.error(
    `The Inboxjot CLI has no build for ${platform}. Install it with Homebrew, or see https://inboxjot.com/docs/cli`
  );
  process.exit(1);
}

const child = spawn(binary, process.argv.slice(2), { stdio: 'inherit' });

process.on('SIGINT', () => {});
for (const signal of ['SIGTERM', 'SIGHUP']) {
  process.on(signal, () => child.kill(signal));
}

child.on('error', (error) => {
  console.error(error.message);
  process.exit(1);
});
child.on('exit', (code, signal) => {
  if (signal) {
    process.removeAllListeners(signal);
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 1);
});
