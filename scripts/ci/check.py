"""Keep the existing required `checks` result strict when target jobs are selected/skipped."""
import json
import os

JOBS = {"native-tests": "native", "windows-tests": "windows", "windows-smoke": "windows",
        "web": "web", "geometry-webassembly": "wasi", "licenses": "licenses"}
ALWAYS = ("changes", "documentation-links", "library-schemas")


def failures(needs: dict) -> list[str]:
    """Selected jobs must succeed; cancelled/failed work and malformed selections never pass."""
    errors = []
    for name in ALWAYS:
        if needs.get(name, {}).get("result") != "success":
            errors.append(f"{name} did not succeed")
    outputs = needs.get("changes", {}).get("outputs", {})
    for key in set(JOBS.values()) | {"windows_full"}:
        if outputs.get(key) not in {"true", "false"}:
            errors.append(f"missing/invalid selection: {key}")
    if outputs.get("windows_full") == "true" and outputs.get("windows") != "true":
        errors.append("full Windows validation was selected without its job")
    for name, selection in JOBS.items():
        result = needs.get(name, {}).get("result")
        allowed = {"success"} if outputs.get(selection) == "true" else {"success", "skipped"}
        if result not in allowed:
            errors.append(f"{name}: {result or 'missing'}")
    return errors


if __name__ == "__main__":
    errors = failures(json.loads(os.environ["CI_NEEDS"]))
    if errors:
        raise SystemExit("\n".join(errors))
    print("All selected checks passed.")
