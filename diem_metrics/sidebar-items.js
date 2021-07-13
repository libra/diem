initSidebarItems({"fn":[["gather_metrics",""],["get_all_metrics",""]],"macro":[["monitor","Helper function to record metrics for external calls. Include call counts, time, and whether it’s inside or not (1 or 0). It assumes a OpMetrics defined as OP_COUNTERS in crate::counters;"],["register_histogram","Create a [`Histogram`] and registers to default registry."],["register_histogram_vec","Create a [`HistogramVec`] and registers to default registry."],["register_int_counter","Create an [`IntCounter`] and registers to default registry."],["register_int_counter_vec","Create an [`IntCounterVec`] and registers to default registry."],["register_int_gauge","Create an [`IntGauge`] and registers to default registry."],["register_int_gauge_vec","Create an [`IntGaugeVec`] and registers to default registry."]],"mod":[["metric_server",""]],"static":[["NUM_METRICS",""]],"struct":[["DurationHistogram","A small wrapper around Histogram to handle the special case of duration buckets. This Histogram will handle the correct granularity for logging time duration in a way that fits the used buckets."],["Histogram","A [`Metric`] counts individual observations from an event or sample stream in configurable buckets. Similar to a `Summary`, it also provides a sum of observations and an observation count."],["HistogramTimer","Timer to measure and record the duration of an event."],["OpMetrics",""]],"type":[["HistogramVec","A [`Collector`] that bundles a set of Histograms that all share the same [`Desc`], but have different values for their variable labels. This is used if you want to count the same thing partitioned by various dimensions (e.g. HTTP request latencies, partitioned by status code and method)."],["IntCounter","The integer version of [`Counter`]. Provides better performance if metric values are all positive integers (natural numbers)."],["IntCounterVec","The integer version of [`CounterVec`]. Provides better performance if metric are all positive integers (natural numbers)."],["IntGauge","The integer version of [`Gauge`]. Provides better performance if metric values are all integers."],["IntGaugeVec","The integer version of [`GaugeVec`]. Provides better performance if metric values are all integers."]]});