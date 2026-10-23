import os
import importlib


class Session:
    def handlers(self):
        from acme.api import handlers  # zenith-ignore core-is-the-base: moving in ENG-42

        return handlers

    def plugin(self, name):
        return importlib.import_module("acme.api.models")
