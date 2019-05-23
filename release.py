import subprocess
import shlex
import sys


def run(cmd: str):
    subprocess.run(shlex.split(cmd), check=True)


def main():
    level = sys.argv[1]
    run("cargo clippy --release")
    run(f"cargo release {level}")


if __name__ == "__main__":
    main()
