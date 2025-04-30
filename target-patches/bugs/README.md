# Bugs

This folder contains patches that re-introduce real bugs into target full node
implementations.

| Patch File                                                                     | Implementation | Bug Type         |
|:-------------------------------------------------------------------------------|:---------------|:-----------------|
| `bitcoin-core/0001-Revert-p2p-don-t-find-1p1cs-for-reconsiderable-txns-.patch` | Bitcoin Core   | Assume crash     |
| `bitcoin-core/0002-p2p-Reintroduce-blocktxn-assertion-crash-bug.patch`         | Bitcoin Core   | Assert crash     |
