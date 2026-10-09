"""Checks the native partitioner's hash against Kafka's own test vectors."""
from lawtwin.kafka.contract import murmur2


def test_murmur2_matches_kafka():
    """The values the Java client's own murmur2 test checks."""
    assert murmur2(b"21") == -973932308
    assert murmur2(b"foobar") == -790332482
    assert murmur2(b"a-little-bit-long-string") == -985981536
    assert murmur2(b"a-little-bit-longer-string") == -1486304829
    assert murmur2(b"lkjh234lh9fiuh90y23oiuhsafujhadof229phr9h19h89h8") == -58897971
    assert murmur2(b"abc") == 479470107
