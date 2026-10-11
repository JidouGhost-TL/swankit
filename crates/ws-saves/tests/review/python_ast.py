"""Serialize synthetic Python syntax; executes no definition code."""
import ast
import json
import sys

def node(n):
    if isinstance(n, ast.Constant) and isinstance(n.value, bytes):
        return {"t": "Bytes", "v": list(n.value)}
    if isinstance(n, ast.AST):
        return dict(t=type(n).__name__, **{k: node(v) for k, v in ast.iter_fields(n)})
    if isinstance(n, bytes):
        return {'bytes': list(n)}
    if isinstance(n, list):
        return [node(v) for v in n]
    return n

print(json.dumps(node(ast.parse(sys.stdin.read()))))
