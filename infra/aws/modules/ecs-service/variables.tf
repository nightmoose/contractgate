variable "name" {
  type        = string
  description = "Service and cluster name prefix. Use the nmb- prefix."
}

variable "cpu" {
  type    = string
  default = "512"
}

variable "memory" {
  type    = string
  default = "1024"
}

variable "container_image" {
  type        = string
  description = "Container image URI (same image as Fly, pointed at staging only)."
}

variable "container_port" {
  type    = number
  default = 8080
}

variable "execution_role_arn" {
  type        = string
  description = "Task execution role ARN (nmb-ecs-task-exec, created in P7)."
}

variable "task_role_arn" {
  type        = string
  description = "Task role ARN (nmb-ecs-task, created in P7)."
}

variable "subnet_ids" {
  type = list(string)
}

variable "security_group_ids" {
  type = list(string)
}

variable "desired_count" {
  type    = number
  default = 1
}

variable "assign_public_ip" {
  type    = bool
  default = true
}

variable "environment" {
  type    = map(string)
  default = {}
}

variable "tags" {
  type    = map(string)
  default = {}
}
