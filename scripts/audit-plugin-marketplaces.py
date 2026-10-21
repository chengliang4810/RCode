"""Read public marketplace manifests; report structural coverage, not service/LLM acceptance.

Never executes downloaded code. Results and downloaded JSON are kept under output/.
"""
import concurrent.futures
import datetime
import json
import pathlib
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / "output" / "marketplace-compatibility"
OUT.mkdir(parents=True, exist_ok=True)
EVENTS = {"SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PostToolUseFailure", "Stop", "StopFailure"}


def fetch(url):
    errors = []
    for proxy in ("https://gh-proxy.com/", "https://ghfast.top/", ""):
        try:
            request = urllib.request.Request(proxy + url, headers={"User-Agent": "RCode-compatibility-audit"})
            with urllib.request.urlopen(request, timeout=10) as response:
                data = response.read(8 * 1024 * 1024 + 1)
            if len(data) > 8 * 1024 * 1024:
                raise ValueError("JSON too large")
            return json.loads(data), None
        except urllib.error.HTTPError as error:
            errors.append(str(error))
        except Exception as error:
            errors.append(str(error))
    return None, "; ".join(errors)


def audit(entry, market):
    source = entry["source"]
    if isinstance(source, str):
        repo, revision, path = market, "main", source.removeprefix("./").rstrip("/")
    else:
        repo = source.get("url", source.get("repo", "")).removeprefix("https://github.com/").removesuffix(".git")
        revision = source.get("sha", source.get("ref", "HEAD"))
        path = source.get("path", "").removeprefix("./").rstrip("/")
    base = f"https://raw.githubusercontent.com/{repo}/{revision}/" + (path + "/" if path else "")
    cache = OUT / market.replace("/", "_") / (entry["name"] + ".json")
    if cache.exists():
        try:
            cached = json.loads(cache.read_text(encoding="utf-8"))
            cached["gaps"] = [gap for gap in cached["gaps"] if not (gap.startswith("hook event: ") and gap.removeprefix("hook event: ") in EVENTS)]
            if not any(gap.startswith("manifest unavailable:") for gap in cached["gaps"]):
                return cached
        except (ValueError, OSError):
            pass
    manifest, error = fetch(base + ".claude-plugin/plugin.json")
    reasons = []
    if manifest is None:
        if entry.get("strict", True):
            reasons.append("manifest unavailable: " + str(error))
        manifest = {key: entry[key] for key in ("commands", "skills", "agents", "hooks", "mcpServers", "lspServers") if key in entry}
    features = set(manifest) & {"commands", "skills", "agents", "hooks", "mcpServers", "lspServers", "outputStyles"}
    if "lspServers" in features:
        reasons.append("LSP")
    if "outputStyles" in features:
        reasons.append("output styles")
    hooks = manifest.get("hooks")
    if isinstance(hooks, str):
        hooks, hook_error = fetch(base + hooks.removeprefix("./"))
        if hooks is None:
            reasons.append("declared hooks unavailable: " + str(hook_error))
    elif hooks is None:
        hooks, _ = fetch(base + "hooks/hooks.json")
    if isinstance(hooks, dict):
        features.add("hooks")
        for event, rules in hooks.get("hooks", {}).items():
            if event not in EVENTS:
                reasons.append("hook event: " + event)
            for rule in rules if isinstance(rules, list) else []:
                for hook in rule.get("hooks", []):
                    if hook.get("type", "command") != "command":
                        reasons.append("hook type: " + str(hook.get("type")))
    result = {"name": entry["name"], "repository": repo, "revision": revision,
              "features": sorted(features), "gaps": sorted(set(reasons)),
              "manifest": manifest, "hooks": hooks, "manifest_url": base + ".claude-plugin/plugin.json"}
    cache.parent.mkdir(parents=True, exist_ok=True)
    cache.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    return result


def main():
    report = {"generated_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "scope": "Manifest/source/hook format audit only; does not verify external authentication, binaries, live service behavior, or model compliance.", "markets": []}
    for repo in ("anthropics/claude-plugins-official", "DietrichGebert/ponytail", "obra/superpowers"):
        catalog, error = fetch(f"https://raw.githubusercontent.com/{repo}/main/.claude-plugin/marketplace.json")
        if catalog is None:
            raise RuntimeError(repo + ": " + error)
        with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
            rows = list(pool.map(lambda entry: audit(entry, repo), catalog["plugins"]))
        compatible = sum(not row["gaps"] for row in rows)
        result = {"repository": repo, "total": len(rows), "without_identified_structural_gaps": compatible,
                  "structural_coverage_percent": round(100 * compatible / len(rows), 2), "plugins": rows}
        report["markets"].append(result)
        print(repo, compatible, "/", len(rows), result["structural_coverage_percent"], flush=True)
    (OUT / "audit.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
