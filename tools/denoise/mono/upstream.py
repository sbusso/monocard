"""Greycard's own modules, one folder up. They carry the names of the
modules here (model, data, train), so they are loaded by path."""

import importlib.util
import pathlib


def load(name: str):
    path = pathlib.Path(__file__).resolve().parents[1] / f"{name}.py"
    spec = importlib.util.spec_from_file_location(f"greycard_{name}", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
