"""Keep the existing required `checks` result strict when target jobs are selected/skipped."""
import json
import os

JOBS = {"native-tests": "native", "windows-tests": "windows", "windows-smoke": "windows",
        "macos-tests": "macos", "web": "web", "geometry-webassembly": "wasi",
        "licenses": "licenses", "quick-compile": "quick"}
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
    mode = outputs.get("mode")
    if mode not in {"docs", "quick", "approved", "full", "carry"}:
        errors.append("missing/invalid CI stage")
    full_keys = set(JOBS.values()) - {"quick"} | {"windows_full"}
    if mode == "quick" and (outputs.get("quick") != "true" or any(outputs.get(key) != "false" for key in full_keys - {"licenses"})):
        errors.append("quick feedback requires compilation and defers heavy checks")
    if mode == "approved" and (outputs.get("native") != "true" or any(outputs.get(key) != "false" for key in (full_keys - {"native", "licenses"}) | {"quick"})):
        errors.append("approved PRs require native checks and defer other platforms")
    if mode == "full" and (any(outputs.get(key) != "true" for key in full_keys) or outputs.get("quick") != "false"):
        errors.append("qualification requires every platform check")
    if mode in {"docs", "carry"} and any(outputs.get(key) != "false" for key in full_keys | {"quick"}):
        errors.append("non-build stages cannot select compiler jobs")
    for name, selection in JOBS.items():
        result = needs.get(name, {}).get("result")
        allowed = {"success"} if outputs.get(selection) == "true" else {"success", "skipped"}
        if result not in allowed:
            errors.append(f"{name}: {result or 'missing'}")
    return errors


if __name__ == "__main__":
    needs = json.loads(os.environ["CI_NEEDS"])
    errors = failures(needs)
    if not errors and needs["changes"]["outputs"]["mode"] == "approved" and os.environ.get("GITHUB_EVENT_NAME") == "pull_request":
        from qualification import require_current_approval
        require_current_approval()
    if not errors and needs["changes"]["outputs"]["mode"] == "carry":
        from qualification import wait_for_pass
        wait_for_pass()
    if errors:
        raise SystemExit("\n".join(errors))
    print("All selected checks passed.")
