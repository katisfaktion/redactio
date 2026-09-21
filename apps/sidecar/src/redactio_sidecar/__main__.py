import sys


def main() -> None:
    if "--manage-models" in sys.argv[1:]:
        from .model_manager import main as run
    else:
        from .ipc import main as run
    run()


if __name__ == "__main__":
    main()
