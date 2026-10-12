{{/*
weebo-authentik's parameters for the kit's library chart
(weebo-operator-lib, see its README): everything templates/operator.yaml
renders that is not a value.
*/}}
{{- define "operator.params" -}}
group: authentik.weebo.io
versions: [v1alpha1]
# Every kind a reconciler watches (cluster-wide, or the namespaced ones in
# watch.namespace only).
kinds:
  - authentikinstances
  - authentiklocalinstances
  - authentikgroups
  - authentikflows
  - authentikoutposts
  - authentikscopemappings
  - authentikusers
  - authentikbrands
  - authentikapplications
  - authentikaccesspolicies
  - authentiknamespacepolicies
# Checked by the AuthentikNamespacePolicy allow-list on admission (unless
# bound to an AuthentikLocalInstance).
admitted:
  - authentikapplications
  - authentikaccesspolicies
# The namespaced kinds: what a team may manage (rbac.namespacedExample),
# and the watch Role's (watch.namespace).
namespacedKinds:
  - authentiklocalinstances
  - authentikapplications
  - authentikaccesspolicies
env: []
egress:
  - comment: Authentik instances.
    rules: {{ toJson .Values.networkPolicy.egress.authentik }}
alerts: []
{{- end -}}
