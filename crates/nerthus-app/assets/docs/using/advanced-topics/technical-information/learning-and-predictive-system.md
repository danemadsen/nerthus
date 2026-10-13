# Learning and Predictive System

Nerthus incorporates a sophisticated, fully-local Learning and Predictive System designed to streamline your workflow by intelligently suggesting nodes based on your historical usage patterns. This system is a key part of Nerthus’s Convenience Features and the Toolbox, which aim to significantly reduce graph creation times.

## How It Works

### Node Usage Tracking

* **Pattern Learning**: As you work, Nerthus monitors the nodes you create and the connections you establish between them. It analyzes these patterns to understand your workflow preferences and tendencies.

### Predictive Suggestions

* **Node Recommendations**: Based on your usage patterns, Nerthus predicts the nodes you are likely to use next and prioritizes these in the node suggestion list.
* **Optional Use**: These predictive suggestions are presented separately from the main Create menu, to ensure they are unobtrusive and can be easily ignored if you prefer the standard node selection methods.

## Predicting Connections

* **Quick Create Menu**: When you initiate a node connection, the Quick Create menu appears, offering suggestions for next nodes based on your previous actions.
* **Example Use Case**: If you frequently connect an Autolevel node to the Flow output of an Erosion node, then starting a connection from the Flow port and dragging it into an empty space on the Graph surface will trigger the Quick Create menu, with Autolevel prominently suggested.
* **Visual Aids**: Suggestions are color-coded to facilitate quick identification of node types, enhancing usability and speed.

## Toolbox

* In the Toolbox, frequently used nodes are highlighted in a brighter color for faster recognition.

## Tip for Enhanced Productivity

Regular interaction with the Predictive System not only speeds up your current projects, but also continuously refines the system's accuracy. As Nerthus learns more about your preferences and habits, the suggestions become increasingly tailored to your specific workflow, potentially cutting graph creation times in half.

## Conclusion

Nerthus's Learning and Predictive System is a powerful tool for enhancing efficiency in terrain creation. By learning from your usage patterns and intelligently predicting your next steps, the system allows for a more streamlined and personalized design experience.

***

## Privacy and Security

* **Local Processing**: All pattern recognition and data processing occur on your local machine. No personal data or usage information is sent to external servers or stored online.
* **Control and Transparency**: Users have complete control over this feature. The toolbox's highlighting and pinning of most-used nodes can be turned off in Options › Toolbox. All data related to predictive learning and node suggestion tracking is kept in `usage.stats` in the [data folder](../../troubleshooting/diagnostics/data-folder.md); delete it to start afresh.
