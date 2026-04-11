# CAP Theorem

Notes on the CAP theorem and its implications for [[distributed-systems]].

## The Three Properties

- **Consistency**: Every read receives the most recent write or an error
- **Availability**: Every request receives a response (no errors), but no guarantee it's the most recent write
- **Partition tolerance**: The system continues to operate despite network partitions between nodes

## The Trade-off

You can only guarantee two of the three. In practice, network partitions are unavoidable in distributed systems, so the real choice is between **CP** (consistent but may be unavailable during partitions) and **AP** (available but may return stale data during partitions).

## Real-World Examples

| System | Type | Notes |
|--------|------|-------|
| ZooKeeper | CP | Strong consistency, may reject writes during partition |
| Cassandra | AP | Tunable consistency, favours availability |
| MongoDB | CP | Single-leader, consistent reads from primary |
| DynamoDB | AP | Eventually consistent by default, optional strong reads |

## PACELC Extension

Eric Brewer later clarified: when there is no Partition, you still face a trade-off between Latency and Consistency. So it's really PA/EL, PC/EC, PA/EC, or PC/EL.

## Questions

- How does CockroachDB claim to be both consistent and available? (Answer: it's CP but with very fast failover, so the "unavailability" window is small)
- What consistency model does our [[search-caching]] use? (Answer: eventual, with 5-minute TTL)
