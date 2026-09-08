# Deployment Modes

## Local experimentation

Recommended backend:

- `in_memory` for quick local request/validation testing.

Characteristics:

- No persistence: data is lost on process restart.
- Single-process state only.
- Not suitable for horizontal scaling or replica failover.
- Supports replay and watch in-process, limited by local memory retention.
- For local JetStream testing, see
  [Installation: Local JetStream](./installation.md#local-jetstream-docker).
- For a quick end-to-end behavior check, see
  [Getting Started: Run the Smoke Test](./getting-started.md#run-the-smoke-test).

## Production-like / persistent mode

Recommended backend:

- `jetstream`

Characteristics:

- Durable message storage.
- Retention and size limits.
- Replica support (requires clustered NATS setup).
- Supports replay and live streaming workflows.

For Kubernetes, use the
[Aviso Helm chart](https://github.com/ecmwf/aviso-chart).

## Selection guideline

- Need persistence/replay/streaming robustness: use `jetstream`.
- Need fastest setup for local functional checks only: use `in_memory`
  (node-local replay/watch).

## Deploying with Helm

The server provides the application and its configuration format. The chart
defines Kubernetes resources and supplies default values. Keep your own
deployment values separately; no additional configuration repository is
required.

### Prepare the chart

You need a Kubernetes cluster, `kubectl`, and Helm with OCI support. The chart
and its dependencies, as well as the server image, must be accessible from
your environment. The default image and one chart dependency are hosted at
`eccr.ecmwf.int`; check registry access before installing. Helm registry
authentication does not supply image-pull credentials to Kubernetes nodes.

To install from a chart checkout:

```bash
git clone https://github.com/ecmwf/aviso-chart.git
helm repo add nats https://nats-io.github.io/k8s/helm/charts/
helm dependency build ./aviso-chart
```

Select the chart revision you intend to deploy before building dependencies.
`helm dependency build` uses the chart's lockfile. Dependencies must be
downloadable even when their components are disabled in your values.

### Start with a development deployment

This example runs one Aviso replica and one NATS server with JetStream.
Your cluster needs a default StorageClass that can provision a persistent
volume. Create `values-demo.yaml`:

```yaml
fullnameOverride: aviso
replicaCount: 1

nats:
  enabled: true
  config:
    jetstream:
      enabled: true
      fileStore:
        enabled: true
        pvc:
          enabled: true
          size: 10Gi

extraEnv:
  - name: AVISOSERVER_CONFIG_FILE
    value: /etc/aviso_server/config.yaml

config:
  notification_backend:
    kind: jetstream
    jetstream:
      nats_url: nats://aviso-nats:4222
      storage_type: file
      replicas: 1
```

The NATS URL assumes the Helm release name `aviso`, used in the commands
below. Change the URL if you choose another release name.

NATS stores notifications on the persistent volume, so they can survive pod
restarts while that volume is retained. This uses chart defaults for the
remaining settings, with no authentication or ingress. It is a development
setup, not a highly available production deployment.

Validate and inspect the rendered resources before installing:

```bash
helm lint ./aviso-chart -f values-demo.yaml
helm template aviso ./aviso-chart --namespace aviso -f values-demo.yaml

helm upgrade --install aviso ./aviso-chart \
  --namespace aviso \
  --create-namespace \
  -f values-demo.yaml \
  --wait --timeout 5m
```

The `fullnameOverride` setting names the Aviso Service and Deployment
`aviso`. For a temporary local check, forward the Service port to your
machine:

```bash
kubectl -n aviso port-forward service/aviso 8000:8000
```

Keep this command running while testing. It exposes Aviso on your machine's
loopback interface and stops forwarding when you press Ctrl+C. It does not
provide a shared endpoint for other users.

In another terminal, check liveness and backend-connection readiness:

```bash
curl --fail http://127.0.0.1:8000/health
curl --fail http://127.0.0.1:8000/ready
```

Applications inside the cluster can use the Kubernetes Service directly.
For access from outside the cluster, configure the chart's ingress settings
with your hostname and TLS configuration, or use your platform's supported
exposure method. An Ingress requires an ingress controller; creating the
resource alone does not make the service reachable. Configure authentication
before exposing Aviso, and ensure proxy buffering and timeouts support
long-lived SSE connections.

### Understand the configuration layers

Helm merges chart defaults with your `-f` files in command-line order. Later
files take precedence. Maps merge, but lists such as `extraEnv` are replaced
as a whole. You can use one values file or layer several; their names and
organization are yours to choose.

Top-level chart values configure Kubernetes resources. The `config:` map
contains server settings and becomes a ConfigMap mounted at
`/etc/aviso_server/config.yaml`.

The server then applies its own
[configuration loading rules](./configuration.md#loading-precedence).
Without an explicit file selector, configuration included in the image can
be merged with the mounted file. Omitting a setting from an overlay does not
remove it from an earlier source.

The `AVISOSERVER_CONFIG_FILE` setting in the example selects the mounted file
as the only file source. Field-level `AVISOSERVER_*` environment variables
still override its settings. This does not remove Helm's chart defaults.

Use Helm values for settings that also affect rendered resources, such as
the application port. Changing the server port only through an environment
variable does not update the chart's container ports or probes.

### Supply credentials through Secrets

Do not put credentials in `config:`: it is stored in a ConfigMap, not a
Secret. Provision Secrets separately and reference their keys through
`extraEnv`. For example, a deployment using token-authenticated NATS could
use this values fragment:

```yaml
extraEnv:
  - name: AVISOSERVER_CONFIG_FILE
    value: /etc/aviso_server/config.yaml
  - name: AVISOSERVER_NOTIFICATION_BACKEND__JETSTREAM__TOKEN
    valueFrom:
      secretKeyRef:
        name: aviso-nats-credentials
        key: token
```

Create that Secret in the release namespace before installation. This
fragment supplies credentials only; it does not select or deploy JetStream.
Keep any other required `extraEnv` entries in the same list.

### Prepare for production

The example configures both sides: `nats.enabled` deploys NATS, while
`config.notification_backend` tells Aviso how to connect to it. Neither
setting automatically configures the other. To use an existing NATS
deployment, disable the dependency and supply its URL and credentials.

Choose retention and capacity limits for your notification schemas through
`storage_policy`. Schema settings override backend defaults per field.
The demo intentionally leaves retention unspecified rather than choosing
one age limit for every stream. Other limits still apply, including keeping
only the latest notification per subject by default. See the
[JetStream backend guide](./backend-jetstream.md) for storage policy details.

Aviso pod replicas and JetStream stream replicas are separate settings.
A single NATS server with persistent storage is not a highly available
cluster. Before exposing the service, also configure
[authentication](./authentication.md) and ingress/TLS for your environment.

### Apply configuration changes

Use `helm upgrade` with your chosen values files. The chart includes a
checksum of the rendered ConfigMap in the pod template, so configuration
changes through Helm trigger a rollout. The server reads configuration at
startup; this is not live reload.

Editing the ConfigMap directly does not update that checksum. The mounted
file also uses `subPath`, so existing containers do not receive projected
ConfigMap updates. Keep configuration changes in your Helm values.

Updating a referenced Secret's contents does not automatically restart pods.
After rotating credentials, restart the Deployment so its environment
variables are loaded again:

```bash
kubectl -n aviso rollout restart deployment/aviso
kubectl -n aviso rollout status deployment/aviso
```
