# Build Reports

## Post-Action Screen

Nerthus shows a post-action screen when a build finishes. It saves a copy in the build folder as `report.txt` so you can refer to it later.

```

───────────────────────────────────────────────────────────── NERTHUS BUILD  0.1.0 ──

          Filename:  │ C:/Users/you/Documents/Nerthus/Projects/mesher2.terrain  
    Build Location:  │ C:\Users\you\Documents\Nerthus\Builds\mesher2\001        
        Build Type:  │ Standard                                              
  Build Resolution:  │ 2048 × 2048                                           
     Build Profile:  │ None / Last Saved Settings                            
             Nodes:  │ 03 to build                                           
                     │ 01 to export

───────────────────────────────────────────────────────────────────── Build Report ──

              Build Results                              Statistics               

          Result: │ Success                  Sandstone │ 0:00:02.721              
      Resolution: │ 2048                               │                          
      Node Count: │ 3                          Fastest │ Mesher @ 0:00:00.001     
      Start Time: │ 02/05/2025 19:42:03        Slowest │ Sandstone @ 0:00:02.721  
        End Time: │ 02/05/2025 19:42:26                                           
        Duration: │ 00:00:23.3091200       * nodes that took <100ms are excluded. 
  Non-Build Time: │ 00:00:20.5231200                                              

```

## Build Report

A structured data version of the report is also saved as `report.json` that is both human and machine readable.

```json
{
  "$id": "1",
  "Resolution": 2048,
  "NodeCount": 3,
  "Result": "Success",
  "StartTime": "2025-02-05T19:42:03.2694437+05:30",
  "EndTime": "2025-02-05T19:42:26.5785637+05:30",
  "Duration": "00:00:23.3091200",
  "SecondsTaken": {
    "$id": "2",
    "$values": [
      {
        "$id": "3",
        "Item1": "Perlin",
        "Item2": "00:00:00.0640000"
      },
      {
        "$id": "4",
        "Item1": "Mesher",
        "Item2": "00:00:00.0010000"
      },
      {
        "$id": "5",
        "Item1": "Sandstone",
        "Item2": "00:00:02.7210000"
      }
    ]
  },
  "FastestNode": {
    "$ref": "4"
  },
  "SlowestNode": {
    "$ref": "5"
  }
}
```
