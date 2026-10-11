{{/*
weebo-authentik's parameters for the kit's library chart
(weebo-operator-lib, see its README): everything templates/operator.yaml
renders that is not a value.
*/}}
{{- define "operator.params" -}}
group: authentik.weebo.io
versions: [v1alpha1]
# Every reconciler watches its kind cluster-wide, whether that kind is
# cluster- or namespace-scoped.
kinds:
  - authentikinstances
  - authentikgroups
  - authentikflows
  - authentikoutposts
  - authentikscopemappings
  - authentikusers
  - authentikbrands
  - authentikapplications
  - authentikaccesspolicies
  - authentiknamespacepolicies
# The namespaced kinds: checked by the AuthentikNamespacePolicy allow-list
# on admission, and what a team may manage (rbac.namespacedExample).
admitted: &namespaced
  - authentikapplications
  - authentikaccesspolicies
namespacedKinds: *namespaced
env: []
egress:
  - comment: Authentik instances.
    rules: {{ toJson .Values.networkPolicy.egress.authentik }}
alerts: []
{{- end -}}
