"""Reject emitted Cargo compiler warnings, including local dependency diagnostics."""
import json
import subprocess
import sys


def main(command):
    failed = False
    with subprocess.Popen(command, stdout=subprocess.PIPE, encoding='utf-8') as process:
        for line in process.stdout:
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                # Preserve unexpected output and reject incomplete diagnostic evidence.
                sys.stderr.write(line)
                failed = True
                continue
            if record.get('reason') == 'compiler-message':
                message = record['message']
                sys.stderr.write(message.get('rendered') or message['message'] + '\n')
                if message['level'] == 'warning':
                    failed = True
        status = process.wait()
    if status:
        return status
    if failed:
        print('Compiler warning check failed; review the diagnostics above.', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
