# Scale and Resolution

## Nerthus Dimensions and Scale: A Beginner's Guide

Understanding dimensions and scale in Nerthus is essential for building terrains that look realistic and match your project needs. Here’s a simplified overview of key concepts:

> [!NOTE]
> For detailed technical information, see [Dimensions and Scale](../../advanced-topics/technical-information/dimensions-and-scale.md).

### **World Space**

* **What It Is**: World Space refers to the area where your terrain exists.
* **Units of Measurement**: Nerthus typically uses meters (m) to represent distances in World Space. One unit in Nerthus equals one meter.
* **Why It Matters**: Knowing the scale of your terrain helps in creating realistic landscapes, especially when integrating into game engines or 3D software.

### **Resolution**

* **What It Is**: Resolution is the level of detail in your terrain, measured in pixels.
* **Impact of Resolution**: Higher resolutions provide more detail but require more memory and processing power.
* **Common Resolutions**: Nerthus commonly supports resolutions from 512x512 to 8192x8192 pixels. For beginners, start with 1024x1024 or 2048x2048 pixels.

### **Scale and Extents**

* **Scale**: Determines the overall size of the terrain in meters. For example, if your terrain is 1000x1000 units in Nerthus, it represents a 1-kilometer square area.
* **Extent**: The physical area covered by your terrain. A larger extent means a larger physical space, which affects how objects appear in relation to each other.

### **Height Range**

* **What It Is**: Defines the lowest and highest points of the terrain.
* **Setting Heights**: When setting terrain height, think about the kind of landscape you’re creating. For example, 100 meters might work for hills, while 3000 meters suit mountain ranges.
* **Height Precision**: Nerthus allows precise control of height, which is critical for realistic landscapes.

### **Real-World Scaling**

* **Importance**: When integrating terrains into real-world projects, make sure the scaling matches. This is especially vital for game environments and simulations where a terrain needs to match character or object sizes accurately.

### **Vertical Scale**

* **Definition**: Adjusts the height of the terrain relative to its width and depth.
* **Why Adjust**: To create exaggerated terrains, like dramatic mountain ranges, adjust the vertical scale for added effect.

***

## **Quick Tips for Beginners:**

* **Start Small**: Begin with lower resolutions to understand the basics without taxing your computer.
* **Check Compatibility**: Ensure your terrain scale matches the requirements of your target game engine or software.
* **Experiment with Extents and Scale**: These two settings can drastically change how your terrain feels. Try different combinations to get comfortable.

***

This guide provides the fundamentals to begin working with dimensions and scale in Nerthus. As you gain experience, exploring these settings further will enhance your terrain design skills.

For detailed technical information see [Dimensions and Scale](../../advanced-topics/technical-information/dimensions-and-scale.md).
