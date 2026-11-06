"""Reuse an explicitly configured subscription token in process memory only."""
import os
import re
import subprocess


def subscription_environment(env, host="", run=subprocess.run):
    result = dict(env)
    token = result.get("CLAUDE_CODE_OAUTH_TOKEN", "")
    if not token and host:
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.@-]{0,199}", host):
            raise ValueError("invalid_auth_host")
        direct = {k: v for k, v in env.items()
                  if k.lower() not in ("http_proxy", "https_proxy", "all_proxy")}
        script = "import json,pathlib,sys; p=pathlib.Path.home()/'.claude/settings.json'; v=json.loads(p.read_text()).get('env',{}).get('CLAUDE_CODE_OAUTH_TOKEN',''); sys.stdout.write(v)"
        response = run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8",
                        host, "python3 -"], input=script, text=True,
                       capture_output=True, env=direct, timeout=15)
        if response.returncode:
            raise ValueError("subscription_source_unavailable")
        token = response.stdout.strip()
    if not token:
        if host:
            raise ValueError("subscription_token_missing")
        return result, False
    if not re.fullmatch(r"sk-ant-oat[A-Za-z0-9_.-]{16,}", token):
        raise ValueError("invalid_subscription_token")
    result["CLAUDE_CODE_OAUTH_TOKEN"] = token
    result["ANTHROPIC_AUTH_TOKEN"] = ""
    result["ANTHROPIC_API_KEY"] = ""
    result["ANTHROPIC_BASE_URL"] = "https://api.anthropic.com"
    for key in ("CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX", "CLAUDE_CODE_USE_FOUNDRY"):
        result[key] = "0"
    return result, True


if __name__ == "__main__":
    # Auth status never includes the token; useful for a read-only launch check.
    env, enabled = subscription_environment(os.environ, os.environ.get("HERDR_LOCAL_CLAUDE_AUTH_HOST", ""))
    print("subscription source available" if enabled else "using existing CLI authentication")
