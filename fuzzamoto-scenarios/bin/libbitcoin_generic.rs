use fuzzamoto::{
    fuzzamoto_main,
    scenarios::{
        Scenario, ScenarioInput, ScenarioResult, generic::TestCase,
        libbitcoin_generic::LibbitcoinGenericScenario,
    },
    targets::LibbitcoinTarget,
};

fuzzamoto_main!(
    LibbitcoinGenericScenario::<fuzzamoto::connections::V1Transport, LibbitcoinTarget>,
    TestCase
);
