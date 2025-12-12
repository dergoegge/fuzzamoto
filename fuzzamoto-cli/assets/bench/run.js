(async function renderRun() {
  const status = document.getElementById("status");
  const coverageDiv = document.getElementById("coverage");
  const corpusDiv = document.getElementById("corpus");
  const edgeDiv = document.getElementById("edge_hist");
  const relcovDiv = document.getElementById("relcov");

  const setStatus = (msg) => {
    status.textContent = msg;
  };

  try {
    const res = await fetch("report_data.json");
    if (!res.ok) {
      throw new Error("report_data.json not found");
    }
    const data = await res.json();
    const series = data.series || [];
    const summary = data.summary || {};

    if (!series.length) {
      setStatus("No samples found (stats.csv missing or empty).");
      return;
    }

    setStatus("");

    const coverageTraces = series.map((s) => ({
      x: s.elapsed,
      y: s.coverage,
      mode: "lines",
      name: s.cpu,
    }));
    Plotly.newPlot(
      coverageDiv,
      coverageTraces,
      {
        title: "Coverage (%) vs time",
        xaxis: { title: "Elapsed (s)" },
        yaxis: { title: "Coverage (%)" },
        legend: { orientation: "h" },
      },
      { responsive: true }
    );

    const corpusTraces = series.map((s) => ({
      x: s.elapsed,
      y: s.corpus,
      mode: "lines",
      name: s.cpu,
    }));
    Plotly.newPlot(
      corpusDiv,
      corpusTraces,
      {
        title: "Corpus size vs time",
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
          title: "Edge histogram",
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
          title: "Per-CPU relative coverage (%)",
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
