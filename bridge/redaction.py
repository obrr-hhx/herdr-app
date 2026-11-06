"""Shared credential redaction for management evidence; no secret is persisted."""
import re

def redact(value):
    if isinstance(value,str):
        value=re.sub(r'-----BEGIN [^-]*PRIVATE KEY-----[\s\S]*?(?:-----END [^-]*PRIVATE KEY-----|$)','[private key redacted]',value)
        value=re.sub(r'https?://[^\s<>"\']*(?:oauth|authorize)[^\s<>"\']*','[authorization URL redacted]',value,flags=re.I)
        value=re.sub(r'\b(?:sk-|gh[opusr]_|github_pat_|xox[abceoprs]-)[A-Za-z0-9_-]{16,}','[credential redacted]',value)
        value=re.sub(r'Bearer\s+[A-Za-z0-9._~-]{8,}','Bearer [credential redacted]',value,flags=re.I)
        value=re.sub(r'((?:API[_-]?KEY|AUTH[_-]?TOKEN|ACCESS[_-]?TOKEN|REFRESH[_-]?TOKEN|PASSWORD)\s*[=:]\s*)[^\s,;]{8,}',r'\1[credential redacted]',value,flags=re.I)
        return value
    if isinstance(value,list):return [redact(x) for x in value]
    if isinstance(value,dict):return {k:'[credential redacted]' if re.fullmatch(r'access_token|refresh_token|api[_-]?key|password|authorization|auth_token',k,re.I) else redact(v) for k,v in value.items()}
    return value
