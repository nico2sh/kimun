# Distributed Systems

Reading notes from "Designing Data-Intensive Applications" and related materials.

## Key Concepts

### Replication
- Leader-based replication: one writable leader, multiple read replicas
- Multi-leader: useful for multi-datacenter setups
- Leaderless: Dynamo-style, quorum reads/writes

### Partitioning
- **Hash partitioning**: consistent hashing distributes data evenly
- **Range partitioning**: good for range queries, risk of hot spots
- **Consistent hashing**: nodes arranged in a ring, minimal redistribution on changes

### Consensus
- Raft: leader election + log replication
- Paxos: classic but harder to understand
- ZAB (Zookeeper): similar to Raft

## Questions to Explore

- How does CockroachDB handle cross-range transactions?
- What are the trade-offs between Raft and Paxos in practice?
- Connection to [[cap-theorem]] notes
