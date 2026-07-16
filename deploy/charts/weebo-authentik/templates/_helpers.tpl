{{/*
Chart name and version as used by the chart label.
*/}}
{{- define "weebo-authentik.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
Full release name.
*/}}
{{- define "weebo-authentik.fullname" -}}
{{- if contains .Chart.Name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{/*
Common labels.
*/}}
{{- define "weebo-authentik.labels" -}}
helm.sh/chart: {{ include "weebo-authentik.chart" . }}
{{ include "weebo-authentik.selectorLabels" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{/*
Selector labels.
*/}}
{{- define "weebo-authentik.selectorLabels" -}}
app.kubernetes.io/name: {{ .Chart.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/*
ServiceAccount name.
*/}}
{{- define "weebo-authentik.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "weebo-authentik.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{/*
Webhook TLS Secret name (produced by the cert-manager Certificate).
*/}}
{{- define "weebo-authentik.webhookTlsSecretName" -}}
{{- printf "%s-webhook-tls" (include "weebo-authentik.fullname" .) -}}
{{- end -}}
