import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, copyFileSync, cpSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const [version, artifactsDir, outDir] = process.argv.slice(2);
if (!version || !artifactsDir || !outDir) {
  console.error('usage: node .github/scripts/generate-packages.ts <version> <artifacts-dir> <out-dir>');
  process.exit(2);
}

const root = new URL('../..', import.meta.url).pathname;
const cli: { optionalDependencies: Record<string, string>; [key: string]: unknown } = JSON.parse(readFileSync(join(root, 'npm', 'package.json'), 'utf8'));
const tarballs: Record<string, string> = {};

function pack(dir: string): string {
  const output = execFileSync('npm', ['pack', '--json', '--pack-destination', resolve(outDir)], { cwd: dir });
  const [{ filename }]: { filename: string }[] = JSON.parse(output.toString());
  return join(resolve(outDir), filename);
}

function hash(file: string): string {
  return createHash('sha256').update(readFileSync(file)).digest('hex');
}

function writeManifest(dir: string, manifest: object) {
  writeFileSync(join(dir, 'package.json'), `${JSON.stringify(manifest, null, 2)}\n`);
}

mkdirSync(outDir, { recursive: true });

for (const name of Object.keys(cli.optionalDependencies)) {
  const platform = name.replace('@inboxjot/cli-', '');
  const [os, cpu] = platform.split('-');
  const extension = os === 'win32' ? '.exe' : '';
  const dir = join(outDir, 'packages', `cli-${platform}`);
  mkdirSync(join(dir, 'bin'), { recursive: true });

  const binary = join(dir, 'bin', `inboxjot${extension}`);
  copyFileSync(join(artifactsDir, platform, `inboxjot${extension}`), binary);
  chmodSync(binary, 0o755);
  copyFileSync(join(root, 'LICENSE'), join(dir, 'LICENSE'));

  writeManifest(dir, {
    name,
    version,
    description: `The ${platform} binary of the Inboxjot CLI`,
    homepage: cli.homepage,
    license: cli.license,
    author: cli.author,
    repository: cli.repository,
    os: [os],
    cpu: [cpu],
    ...(os === 'linux' && { libc: ['glibc'] }),
    files: ['bin'],
  });
  tarballs[platform] = pack(dir);
}

const cliDir = join(outDir, 'packages', 'cli');
cpSync(join(root, 'npm'), cliDir, { recursive: true });
copyFileSync(join(root, 'README.md'), join(cliDir, 'README.md'));
copyFileSync(join(root, 'LICENSE'), join(cliDir, 'LICENSE'));
writeManifest(cliDir, {
  ...cli,
  version,
  optionalDependencies: Object.fromEntries(
    Object.keys(cli.optionalDependencies).map((name) => [name, version])
  ),
});
tarballs.cli = pack(cliDir);

const formula = readFileSync(join(root, '.github', 'scripts', 'inboxjot.rb'), 'utf8')
  .replaceAll('{{version}}', version)
  .replace(/\{\{sha256:([a-z0-9-]+)\}\}/g, (_match, platform: string) => hash(tarballs[platform]));
writeFileSync(join(outDir, 'inboxjot.rb'), formula);

writeFileSync(join(outDir, 'tarballs.json'), `${JSON.stringify(tarballs, null, 2)}\n`);
console.log(JSON.stringify(tarballs, null, 2));
