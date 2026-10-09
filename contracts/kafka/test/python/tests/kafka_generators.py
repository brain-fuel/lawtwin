"""Generators for the Kafka contract's laws.

Most of the texts a law draws are topic names, which Kafka allows only from
ASCII letters, digits, dots, underscores and dashes, and most of the 32-bit
integers it draws are partition counts, which are small. Drawn from all
texts and all integers, almost no case would pass the laws' refinements. So
three draws in four are names and small counts; the fourth is drawn from
the whole type, so invalid names and out-of-range counts are still tried. A drawn text is
never empty: the laws that need the empty name write it out.
"""
from hypothesis import strategies as _strategies


def _mostly(likely, anything):
    """`likely` three times in four, `anything` otherwise."""
    return _strategies.integers(min_value=0, max_value=3).flatmap(
        lambda pick: anything if pick == 0 else likely)


def names() -> _strategies.SearchStrategy:
    """Topic-name-like texts, mostly, and any text otherwise."""
    return _mostly(_strategies.from_regex(r"[a-zA-Z0-9._-]{1,40}", fullmatch=True),
                   _strategies.text(min_size=1))


def counts() -> _strategies.SearchStrategy:
    """Counts from 1 to 16, mostly, and any Int32 otherwise."""
    return _mostly(_strategies.integers(min_value=1, max_value=16),
                   _strategies.integers(min_value=-2**31, max_value=2**31 - 1))
