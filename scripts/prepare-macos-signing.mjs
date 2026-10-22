import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

if (process.platform === 'darwin') {
    const root = fileURLToPath(new URL('../', import.meta.url));
    const metadata = JSON.parse(execFileSync('cargo', [
        'metadata', '--no-deps', '--format-version', '1',
        '--manifest-path', path.join(root, 'src-tauri/Cargo.toml'),
    ], { encoding: 'utf8' }));
    const profile = process.env.TAURI_ENV_DEBUG === 'true' ? 'debug' : 'release';
    const triple = process.env.TAURI_ENV_TARGET_TRIPLE;
    const directories = [
        ...(triple ? [path.join(metadata.target_directory, triple, profile)] : []),
        path.join(metadata.target_directory, profile),
    ];
    const binary = directories.map(dir => path.join(dir, 'adConverter-cli')).find(existsSync);
    if (!binary) {
        throw new Error(`Missing adConverter-cli in ${directories.join(', ')}`);
    }

    // Tauri signs the main executable before the CLI. codesign validates sibling
    // executables at that point, so an unsigned Intel CLI makes that step fail.
    // Bootstrap it with an ad-hoc signature; Tauri subsequently replaces this
    // with the Developer ID signature before signing and notarizing the app.
    execFileSync('codesign', ['--force', '--sign', '-', binary], { stdio: 'inherit' });
    execFileSync('codesign', ['--verify', '--strict', binary], { stdio: 'inherit' });
}
