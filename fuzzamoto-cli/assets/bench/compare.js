(async function renderCompare() {
  const status = document.getElementById("status");
  const coverageDiv = document.getElementById("coverage");
  const corpusDiv = document.getElementById("corpus");

  const setStatus = (msg) => {
    status.textContent = msg;
  };

  try {
    const res = await fetch("compare_data.json");
    if (!res.ok) {
      throw new Error("compare_data.json not found");
    }
    const data = await res.json();
    const base = data.baseline || {};
    const cand = data.candidate || {};

    if (!base.elapsed || !base.elapsed.length || !cand.elapsed || !cand.elapsed.length) {
      setStatus("No samples found for comparison.");
      return;
    }

    setStatus(data.mode === "suite" ? "Suite mean curves" : "Run curves");

    const baselineName = data.baseline_label || "baseline";
    const candidateName = data.candidate_label || "candidate";

    Plotly.newPlot(
      coverageDiv,
      [
        { x: base.elapsed, y: base.coverage_mean, mode: "lines", name: baselineName },
        { x: cand.elapsed, y: cand.coverage_mean, mode: "lines", name: candidateName },
      ],
      {
        title: "Coverage (%) vs time",
        xaxis: { title: "Elapsed (s)" },
        yaxis: { title: "Coverage (%)" },
        legend: { orientation: "h" },
      },
      { responsive: true }
    );

    Plotly.newPlot(
      corpusDiv,
      [
        { x: base.elapsed, y: base.corpus_mean, mode: "lines", name: baselineName },
        { x: cand.elapsed, y: cand.corpus_mean, mode: "lines", name: candidateName },
      ],
      {
        title: "Corpus size vs time",
        xaxis: { title: "Elapsed (s)" },
        yaxis: { title: "Corpus size" },
        legend: { orientation: "h" },
      },
      { responsive: true }
    );
  } catch (err) {
    setStatus(`Failed to load data: ${err.message}`);
    console.error(err);
  }
})();
