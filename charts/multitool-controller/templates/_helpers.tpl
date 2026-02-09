{{/*
Expand the name of the chart.
*/}}
{{- define "multitool-controller.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "multitool-controller.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "multitool-controller.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels.
*/}}
{{- define "multitool-controller.labels" -}}
helm.sh/chart: {{ include "multitool-controller.chart" . }}
{{ include "multitool-controller.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels.
*/}}
{{- define "multitool-controller.selectorLabels" -}}
app.kubernetes.io/name: {{ include "multitool-controller.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use.
*/}}
{{- define "multitool-controller.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "multitool-controller.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Controller image tag.
*/}}
{{- define "multitool-controller.controllerTag" -}}
{{- default .Chart.AppVersion .Values.controller.image.tag }}
{{- end }}

{{/*
Sidecar image tag.
*/}}
{{- define "multitool-controller.sidecarTag" -}}
{{- default .Chart.AppVersion .Values.sidecar.image.tag }}
{{- end }}
