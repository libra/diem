# Copyright (c) The Diem Core Contributors
# SPDX-License-Identifier: Apache-2.0

import json, random, subprocess, time

FORGE_K8S_CLUSTERS = ["forge-1"]

# ================ Kube job ================
def create_forge_job(context, user, tag, timeout_secs, forge_envs, forge_args):
    job_name = f"forge-{user}-{int(time.time())}"
    job_name = job_name.replace("_", "-")  # underscore not allowed in pod name

    # job template to spin up. Edit this in place
    template = json.loads(
        subprocess.check_output(
            [
                "kubectl",
                "-o=json",
                f"--context={context}",
                "get",
                "job",
                "--selector=app.kubernetes.io/name=forge-debug",
            ],
            stderr=subprocess.DEVNULL,
            encoding="UTF-8",
        )
    )
    if len(template["items"]) != 1:
        print("ERROR: there must be exactly one forge-debug job")
        return None

    template = template["items"][0]

    # delete old spec details
    del template["metadata"]["selfLink"]
    del template["spec"]["selector"]["matchLabels"]["controller-uid"]
    del template["spec"]["template"]["metadata"]["labels"]["controller-uid"]
    del template["spec"]["template"]["metadata"]["labels"]["job-name"]
    # change job name, labels, and backoff limit
    template["metadata"]["name"] = job_name
    template["metadata"]["labels"]["app.kubernetes.io/name"] = "forge"
    template["spec"]["template"]["metadata"]["labels"][
        "app.kubernetes.io/name"
    ] = "forge"
    template["spec"]["backoffLimit"] = 0
    # change startup command with timeout and extra args
    cmd = template["spec"]["template"]["spec"]["containers"][0]["command"][2]
    template["spec"]["template"]["spec"]["containers"][0]["command"][2] = cmd.replace(
        "tail -f /dev/null",
        f"timeout {timeout_secs} forge {' '.join(forge_args)}".strip(),
    )
    # additional environment variables
    for env_var in forge_envs:
        name, value = env_var.split("=")
        template["spec"]["template"]["spec"]["containers"][0]["env"].append(
            {"name": name, "value": value}
        )
    # new image tag
    image_repo, _ = template["spec"]["template"]["spec"]["containers"][0][
        "image"
    ].split(":")
    template["spec"]["template"]["spec"]["containers"][0][
        "image"
    ] = f"{image_repo}:{tag}"
    return job_name, template


# ================ Kube queries ================


def get_cluster_context(cluster_name):
    k8s_context_pattern = "arn:aws:eks:us-west-2:853397791086:cluster/libra-CLUSTERNAME"
    return k8s_context_pattern.replace("CLUSTERNAME", cluster_name)


# randomly select a cluster that is free based on its pod status:
# - no other forge pods currently Running or Pending
# - all monitoring pods are ready
def kube_select_cluster():
    shuffled_clusters = random.sample(FORGE_K8S_CLUSTERS, len(FORGE_K8S_CLUSTERS))
    attempts = 360
    for attempt in range(attempts):
        for cluster in shuffled_clusters:
            context = get_cluster_context(cluster)
            running_pods = get_forge_pods_by_phase(context, "Running")
            pending_pods = get_forge_pods_by_phase(context, "Pending")
            monitoring_pods = get_monitoring_pod(context)

            # check pod status
            num_running_pods = len(running_pods["items"])
            num_pending_pods = len(pending_pods["items"])
            for pod in monitoring_pods["items"]:
                pod_name = pod["metadata"]["name"]
                healthy = all(
                    list(
                        map(
                            lambda container: container["ready"],
                            pod["status"]["containerStatuses"],
                        )
                    )
                )
                if not healthy:
                    print(
                        f"{cluster} has an unhealthy monitoring pod {pod_name}. Skipping."
                    )
                    continue

            if num_running_pods > 0:
                print(f"{cluster} has {num_running_pods} running forge pods. Skipping.")
            elif num_pending_pods > 0:
                print(f"{cluster} has {num_pending_pods} pending forge pods. Skipping.")
            else:
                return cluster

        print(
            f"All clusters have jobs running on them. Retrying in 10 secs. Attempt: {attempt}/{attempts}"
        )
        time.sleep(10)
    print("Failed to schedule forge pod. All clusters are busy")
    return None


def kube_wait_job(job_name, context):
    attempts = 360
    for _ in range(attempts):
        try:
            phase = get_forge_job_phase(job_name, context)
        except subprocess.CalledProcessError:
            print(f"kubectl get pod {job_name} failed. Retrying.")
            continue

        # pod is either Running, Succeeded, or assume it's working
        # https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/#pod-phase
        if phase in ["Running", "Succeeded", "Unknown"]:
            print(f"{job_name} reached phase: {phase}")
            return 0

        if phase in ["Failed"]:
            print(f"{job_name} reached phase: {phase}")
            return 1

        # error pulling the image
        ret = subprocess.call(
            f"kubectl --context='{context}' get pod --selector=job-name={job_name} | grep -i -e ImagePullBackOff -e InvalidImageName -e ErrImagePull",
            shell=True,
            # stdout=subprocess.DEVNULL,
            # stderr=subprocess.DEVNULL,
        )
        if ret == 0:
            image_name = get_forge_image_name(job_name, context)
            print(
                f"Job {job_name} failed to be scheduled because there was an error pulling the image: {image_name}"
            )
            subprocess.call(
                ["kubectl", f"--context={context}", "delete", "job", job_name]
            )
            return 1

        print(f"Waiting for {job_name} to be scheduled. Current phase: {phase}")
        time.sleep(1)

    print(f"Failed to schedule job: {job_name}")
    return 1


# init the kube context for each available cluster
def kube_init_context():
    try:
        subprocess.run(
            [
                "aws",
                "eks",
                "--region",
                "us-west-2",
                "describe-cluster",
                "--name",
                f"libra-{FORGE_K8S_CLUSTERS[0]}",
            ],
            stdout=subprocess.DEVNULL,
        )
    except subprocess.CalledProcessError:
        print("Failed to access EKS, try awsmfa?")
        raise
    for cluster in FORGE_K8S_CLUSTERS:
        subprocess.run(
            [
                "aws",
                "eks",
                "--region",
                "us-west-2",
                "update-kubeconfig",
                "--name",
                f"libra-{cluster}",
            ]
        )


# ================ Internal helpers ================


def get_forge_pods_by_phase(context, phase):
    return json.loads(
        subprocess.check_output(
            [
                "kubectl",
                "-o=json",
                f"--context={context}",
                "get",
                "pods",
                "--selector=app.kubernetes.io/name=forge",
                f"--field-selector=status.phase=={phase}",
            ],
            stderr=subprocess.DEVNULL,
            encoding="UTF-8",
        )
    )


def get_monitoring_pod(context):
    return json.loads(
        subprocess.check_output(
            [
                "kubectl",
                "-o=json",
                f"--context={context}",
                "get",
                "pods",
                "--selector=app.kubernetes.io/name=monitoring",
            ],
            stderr=subprocess.DEVNULL,
            encoding="UTF-8",
        )
    )


def get_forge_image_name(job_name, context):
    return get_forge_job_jsonpath(
        job_name, context, "{.items[0].spec.containers[0].image}"
    )


def get_forge_job_phase(job_name, context):
    return get_forge_job_jsonpath(job_name, context, "{.items[0].status.phase}")


def get_forge_job_jsonpath(job_name, context, jsonpath):
    return subprocess.check_output(
        [
            "kubectl",
            f"--context={context}",
            "get",
            "pod",
            f"--selector=job-name={job_name}",
            "-o",
            f"jsonpath={jsonpath}",
        ],
        encoding="UTF-8",
    )
