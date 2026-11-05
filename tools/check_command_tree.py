#!/usr/bin/env python3
"""Check /show suggestions against the actual Horizon dispatcher from a native build."""
import argparse
import pathlib
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--deps', required=True, type=pathlib.Path,
                        help='Native Cargo profile/deps directory containing rlibs')
    parser.add_argument('--rustc', default='rustc', help='Compiler used for the native build')
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parent
    libraries = []
    for crate in ['horizon_command', 'lias_media_player']:
        candidates = list(args.deps.glob(f'lib{crate}-*.rlib'))
        if not candidates:
            parser.error(f'Build the native client first: no {crate} rlib in {args.deps}')
        library = max(candidates, key=lambda p: p.stat().st_mtime_ns)
        libraries.extend(['--extern', f'{crate}={library.resolve()}'])
    with tempfile.TemporaryDirectory(prefix='lmp-command-check-') as output:
        binary = pathlib.Path(output) / 'check-command-tree'
        subprocess.run([args.rustc, '--edition', '2024', str(root / 'check_command_tree.rs'),
                        '-L', f'dependency={args.deps.resolve()}', *libraries,
                        '-o', str(binary)], check=True)
        subprocess.run([str(binary)], check=True)


if __name__ == '__main__':
    main()
