# A/B testing demo

A Rust implementation of the solution on [this page](https://developer.fastly.com/solutions/tutorials/ab-testing/).

Fastly needs to know some things about the tests you want to run:

- A list of tests
- A list of buckets in each tests
- Relative weighting of each bucket

This example assumes that test items are defined in the dictionary in the format like this.

```
{
    "tests": "itemcount, buttonsize",
    "itemcount": {
        "buckets": [ "10", "15" ],
        "weight": "1:1"
    },
    "buttonsize": {
        "buckets": [ "small", "medium", "large" ],
        "weight": "7:3:2"
    }
}
```

Deployed at https://ab.edgecompute.app/ for demo.
