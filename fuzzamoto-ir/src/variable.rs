/// `Variable` represents a variable types in the IR
#[derive(Debug, Clone, PartialEq)]
pub enum Variable {
    Nop, // Output type for no-op instructions

    Bytes,             // Raw bytes
    MsgType,           // p2p message type
    Node,              // Index of a node that exists in the context
    Connection,        // Index of a connection that exists in the context
    ConnectionType,    // Connection type
    Duration,          // Duration of time
    HandshakeParams,   // p2p handshake parameters
    Time,              // Point in time
    Size,              // Size in bytes
    BlockHeight,       // Block height
    CompactFilterType, // Compact filter type

    Scripts, // scriptPubKey, scriptSig, witness
    MutWitnessStack,
    ConstWitnessStack,
    PrivateKey,
    SigHashFlags,

    Txo, // Existing transaction output (maybe confirmed)

    MutTx,          // Mutable transaction
    ConstTx,        // Finalized transaction
    MutTxInputs,    // Mutable tx inputs
    ConstTxInputs,  // Finalized tx inputs
    MutTxOutputs,   // Mutable tx outputs
    ConstTxOutputs, // Finalized tx outputs

    ConstAmount, // bitcoin amount in sats

    MutCmpctBlock,   // compact block still being built
    ConstCmpctBlock, // compact block that we finished building.
    Indexes,
    Nonce,

    TxVersion,
    LockTime,
    Sequence,

    MutInventory,
    ConstInventory,

    MutBlockTransactions,
    ConstBlockTransactions,
    Block,
    Header,

    BlockVersion,

    MutFilterLoad, // Mutable filter (under construction)
    ConstFilterLoad,
    FilterAdd,

    BlockTxnRequestVec,
    BIP152BlockTxReq,
    BIP152BlockTx,

    CoinbaseInput,
    CoinbaseTx,
}
