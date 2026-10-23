from __future__ import annotations

from typing import TYPE_CHECKING

import requests

from acme.core.db import Session

from . import models

if TYPE_CHECKING:
    from acme.billing import Invoice


def handle(session: Session) -> models.User:
    return models.User()


def charge(invoice: Invoice) -> None:
    requests.post("https://billing", json=invoice)
