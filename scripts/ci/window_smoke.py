"""Run the staged native-window regression with an external timeout and a nonzero-test oracle."""
import argparse
from pathlib import Path
import re
import subprocess


def run(executable: Path, timeout: float) -> None:
    """Only the owned child is killed on timeout; zero tests and unsuccessful exits fail."""
    command = [str(executable.resolve()), "--ignored", "--exact", "native_window_lifecycle", "--nocapture", "--test-threads=1"]
    try:
        result = subprocess.run(command, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
    except subprocess.TimeoutExpired as error:
        # subprocess.run kills and waits for this child before raising.
        for output in (error.stdout, error.stderr):
            if output:
                print(output.decode("utf-8", errors="replace") if isinstance(output, bytes) else output)
        raise RuntimeError(f"Native window lifecycle exceeded {timeout:g} seconds") from error
    print(result.stdout)
    print(result.stderr)
    if result.returncode != 0 or not re.search(r"test result: ok\. 1 passed; 0 failed;", result.stdout):
        raise RuntimeError(f"Native window lifecycle did not pass its one test (exit {result.returncode})")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    args = parser.parse_args()
    run(args.executable, 180)
