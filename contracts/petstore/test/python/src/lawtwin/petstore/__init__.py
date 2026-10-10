# Written by `lawtwin import openapi`, and written again on every import.
"""The native handlers of lawtwin.petstore, for running its laws: each answers
through the reference model, held by the case's Service. A connection to
the HTTP twin or to the real service takes its place in conformance runs.
"""
import lawspec_abilities.lawtwin.petstore as abilities


class ReferenceService:
    """What a Service holds: the reference model."""

    def __init__(self) -> None:
        self.model = abilities.Reference({})


def connectService(value0):
    return ReferenceService()


def disconnectService(value0) -> None:
    return None


class _Reference:
    """A native handler that answers every operation through the model."""

    def __init__(self, resource) -> None:
        self.resource = resource

    def __getattr__(self, name):
        return getattr(self.resource.model, name)


class PetApiHandler(_Reference):
    """PetApi, answered by the reference model."""


class StoreApiHandler(_Reference):
    """StoreApi, answered by the reference model."""


class UserApiHandler(_Reference):
    """UserApi, answered by the reference model."""
