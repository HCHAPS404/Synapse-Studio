import { fetchHealth, type HealthAnswer, type HealthResult } from "@/lib/health";

// The health call runs per request; it must not run (or fail) at build time.
export const dynamic = "force-dynamic";

function isAnswer(result: HealthResult): result is HealthAnswer {
  return result.kind === "ok" || result.kind === "degraded";
}

export default async function Home() {
  const result = await fetchHealth();
  const answer = isAnswer(result) ? result : null;
  const healthy = result.kind === "ok";

  return (
    <main style={{ maxWidth: 640, margin: "0 auto", padding: "4rem 1.5rem" }}>
      <h1 style={{ marginBottom: 4 }}>SYNAPSE Studio</h1>
      <p style={{ marginTop: 0, opacity: 0.7 }}>
        Design once. Simulate in 3D. Compile to any machine.
      </p>

      <h2>Core API</h2>
      <p role="status">
        <strong>{healthy ? "Connected" : "Problem"}</strong>
        {" - "}
        {isAnswer(result) ? `status ${result.data.status}` : result.message}
      </p>

      {answer && (
        <dl>
          <dt>API</dt>
          <dd>{answer.baseUrl}</dd>
          <dt>Version</dt>
          <dd>{answer.data.version}</dd>
          <dt>Environment</dt>
          <dd>{answer.data.environment}</dd>
          <dt>Database</dt>
          <dd>
            {answer.data.database.status}
            {answer.data.database.latency_ms !== null &&
              ` (${Math.round(answer.data.database.latency_ms)} ms)`}
          </dd>
        </dl>
      )}
    </main>
  );
}
