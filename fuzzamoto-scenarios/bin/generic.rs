use fuzzamoto::{
    fuzzamoto_main,
    scenarios::{
        Scenario, ScenarioInput, ScenarioResult,
        generic::{GenericScenario, TestCase},
    },
    targets::BitcoinCoreTarget,
};

// Transport type alias based on feature flag
#[cfg(not(feature = "v2transport"))]
type ScenarioTransport = fuzzamoto::connections::V1Transport;
#[cfg(feature = "v2transport")]
type ScenarioTransport = fuzzamoto::connections::V2Transport;

fuzzamoto_main!(
    GenericScenario::<ScenarioTransport, BitcoinCoreTarget>,
    TestCase
);
