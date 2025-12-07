(async function renderSuite() {
  const status = document.getElementById("status");
  const coverageDiv = document.getElementById("suite_coverage");
  const corpusDiv = document.getElementById("suite_corpus");
  const edgeDiv = document.getElementById("suite_edge_hist");
  const relcovDiv = document.getElementById("suite_relcov");

  const setStatus = (msg) => {
    status.textContent = msg;
  };

  try {
    const res = await fetch("suite_report_data.json");
    if (!res.ok) {
      throw new Error("suite_report_data.json not found");
    }
    const data = await res.json();
    const series = data.suite_series || {};
    const summary = data.suite_summary || {};

    if (!series.elapsed || !series.elapsed.length) {
      setStatus("No suite samples found (missing run stats).");
      return;
    }

    setStatus("");

    Plotly.newPlot(
      coverageDiv,
      [
        {
          x: series.elapsed,
          y: series.coverage_mean,
          mode: "lines",
          name: "coverage",
        },
      ],
      {
        title: "Coverage (%) vs time (mean across runs)",
        xaxis: { title: "Elapsed (s)" },
        yaxis: { title: "Coverage (%)" },
        legend: { orientation: "h" },
      },
      { responsive: true }
    );

    Plotly.newPlot(
      corpusDiv,
      [
        {
          x: series.elapsed,
          y: series.corpus_mean,
          mode: "lines",
          name: "corpus",
        },
      ],
      {
        title: "Corpus size vs time (mean across runs)",
        xaxis: { title: "Elapsed (s)" },
        yaxis: { title: "Corpus size" },
        legend: { orientation: "h" },
      },
      { responsive: true }
    );

    if (summary.edge_histogram) {
      Plotly.newPlot(
        edgeDiv,
        [
          {
            type: "bar",
            x: ["1-hit", "2-3 hits", ">=4 hits"],
            y: [
              summary.edge_histogram.hit_1,
              summary.edge_histogram.hit_2_3,
              summary.edge_histogram.hit_ge_4,
            ],
          },
        ],
        {
          title: "Edge histogram (sum across runs)",
          xaxis: { title: "Bucket" },
          yaxis: { title: "Count" },
          legend: { orientation: "h" },
        },
        { responsive: true }
      );
    } else {
      edgeDiv.remove();
    }

    if (summary.per_cpu_relcov) {
      Plotly.newPlot(
        relcovDiv,
        [
          {
            type: "bar",
            x: summary.per_cpu_relcov.map((e) => e.cpu),
            y: summary.per_cpu_relcov.map((e) => e.relcov_pct),
          },
        ],
        {
          title: "Per-CPU relative coverage (%) (mean across runs)",
          xaxis: { title: "CPU" },
          yaxis: { title: "Relcov (%)" },
          legend: { orientation: "h" },
        },
        { responsive: true }
      );
    } else {
      relcovDiv.remove();
    }
  } catch (err) {
    setStatus(`Failed to load data: ${err.message}`);
    console.error(err);
  }
})();
