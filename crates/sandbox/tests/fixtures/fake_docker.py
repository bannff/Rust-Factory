#!/usr/bin/env python3
"""Deterministic fake `docker` CLI for adversarial sandbox tests.

`DockerSandbox::run_cli` spawns the CLI with `.env_clear()`, so no environment
variable can reach this process. State location is therefore derived from the
`--host unix://<socket-path>` argument that the code under test already
passes on every invocation: state lives at `<socket-path>.state.json`, and an
optional per-socket delay knob lives at `<socket-path>.delay` (its contents,
if present, are milliseconds to sleep before the atomic `run` commit, used to
widen an inspect-then-run race window in concurrency tests).

State is a JSON map of container name -> {id, owner, tenant, sandbox, running},
mutated under an flock-guarded lock file so concurrent invocations from
multiple OS processes behave with the same atomic-name-uniqueness guarantee a
real docker daemon provides for `run --name`. This lets tests exercise genuine
TOCTOU races in the caller rather than only unit-level logic.
"""
import fcntl
import json
import os
import secrets
import sys
import time


def resolve_paths(args):
    host_index = args.index("--host")
    endpoint = args[host_index + 1]
    socket_path = endpoint.removeprefix("unix://")
    return socket_path + ".state.json", socket_path + ".delay"


STATE, DELAY_FILE = resolve_paths(sys.argv[1:])
LOCK = STATE + ".lock"


def run_delay_ms():
    try:
        with open(DELAY_FILE) as handle:
            return int(handle.read().strip() or "0")
    except FileNotFoundError:
        return 0


def load():
    if not os.path.exists(STATE):
        return {}
    with open(STATE) as handle:
        content = handle.read().strip()
        return json.loads(content) if content else {}


def save(data):
    tmp = STATE + ".tmp"
    with open(tmp, "w") as handle:
        json.dump(data, handle)
    os.replace(tmp, STATE)


def with_lock(fn):
    with open(LOCK, "a+") as lockfile:
        fcntl.flock(lockfile, fcntl.LOCK_EX)
        try:
            return fn()
        finally:
            fcntl.flock(lockfile, fcntl.LOCK_UN)


def main():
    args = sys.argv[1:]
    assert args[0] == "--host"
    sub = args[2]
    rest = args[3:]

    if sub == "inspect":
        name = rest[-1]
        entry = with_lock(lambda: load().get(name))
        if entry is None:
            sys.stderr.write(f"Error: No such container: {name}\n")
            sys.exit(1)
        running = "true" if entry["running"] else "false"
        sys.stdout.write(
            f"{entry['id']}|{entry.get('owner', '')}|{entry.get('tenant', '')}|"
            f"{entry.get('sandbox', '')}|{running}"
        )
        sys.exit(0)

    if sub == "rm":
        docker_id = rest[-1]

        def remove():
            data = load()
            name = next((k for k, v in data.items() if v["id"] == docker_id), None)
            if name is None:
                return False
            del data[name]
            save(data)
            return True

        if not with_lock(remove):
            sys.stderr.write("Error: No such container\n")
            sys.exit(1)
        sys.exit(0)

    if sub == "stopext":
        # Test-only helper (not a real docker subcommand): flips a container's
        # running flag to simulate external state drift between calls.
        name = rest[-1]

        def stop():
            data = load()
            if name not in data:
                return False
            data[name]["running"] = False
            save(data)
            return True

        sys.exit(0 if with_lock(stop) else 1)

    if sub == "run":
        name = None
        owner = tenant = sandbox = ""
        index = 0
        while index < len(rest):
            token = rest[index]
            if token == "--name":
                name = rest[index + 1]
                index += 2
                continue
            if token == "--label":
                label = rest[index + 1]
                if label.startswith("rust-factory.owner="):
                    owner = label.split("=", 1)[1]
                elif label.startswith("rust-factory.tenant="):
                    tenant = label.split("=", 1)[1]
                elif label.startswith("rust-factory.sandbox="):
                    sandbox = label.split("=", 1)[1]
                index += 2
                continue
            index += 1
        new_id = secrets.token_hex(32)
        delay_ms = run_delay_ms()
        if delay_ms:
            time.sleep(delay_ms / 1000)

        def commit():
            data = load()
            if name in data:
                return None
            data[name] = {
                "id": new_id,
                "owner": owner,
                "tenant": tenant,
                "sandbox": sandbox,
                "running": True,
            }
            save(data)
            return new_id

        result = with_lock(commit)
        if result is None:
            sys.stderr.write(
                f'Error response from daemon: Conflict. The container name "/{name}" '
                "is already in use\n"
            )
            sys.exit(1)
        sys.stdout.write(result)
        sys.exit(0)

    if sub == "exec":
        # rest = ["--workdir", dir, docker_id, program, *args]
        sys.stdout.write("exec-ok")
        sys.exit(0)

    sys.stderr.write(f"unknown subcommand {sub}\n")
    sys.exit(2)


if __name__ == "__main__":
    main()
